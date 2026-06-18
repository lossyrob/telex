//! `telex attach`: the resident holder and answerback drum. Claims an exclusive lease,
//! writes a TTL heartbeat, polls the backend for new messages (optionally adds Postgres
//! LISTEN/NOTIFY push), buffers them, and serves ephemeral `wait` clients over local IPC.
//! Blocks for the mission; releases the lease on shutdown (Ctrl-C or a `detach` signal).

use anyhow::Result;
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, Notify};

use crate::backend::Backend;
use crate::cli::{AttachArgs, Ctx};
use crate::config;
use crate::ipc::{self, Frame, Request};
use crate::model::{now_ms, LeaseClaim, LeaseOutcome, MessageRow};
use crate::output::emit;

#[derive(Clone)]
struct Buffered {
    row: MessageRow,
    buffered_at_ms: i64,
}

struct State {
    queue: Mutex<VecDeque<Buffered>>,
    notify: Notify,
    /// Monotonic "already queued by this holder" guard. The `HashSet::insert` under this lock is the
    /// drain serialization point: concurrent poll/push drains — and a stale drain whose
    /// `fetch_undelivered` snapshot predates a concurrent `mark_delivered` — can never re-queue a
    /// message. Never pruned (that is what keeps the dedup race-free); the durable delivery record,
    /// not this set, is what prevents redelivery across restarts. It grows by one `i64` per distinct
    /// message this holder queues over its lifetime — bounded in practice because holders are
    /// session-bound and restart between sessions; a pinned, very-long-lived holder on a busy
    /// address is the case to watch (the drain logs `seen` size so the growth is observable). A
    /// bounded prune is deferred precisely because a naive one re-opens the TOCTOU. See DECISIONS 0011.
    seen: Mutex<HashSet<i64>>,
    last_heartbeat_ms: AtomicI64,
    keepalive: Duration,
    shutdown: Notify,
    backend: Arc<dyn Backend>,
    address: String,
    occupant: String,
}

/// Drain the backend's undelivered set into the local queue. Delivery state — not a monotonic id
/// cursor — decides what is queued: `fetch_undelivered` returns every message addressed here with no
/// delivery record and a non-terminal disposition, so a Postgres id that committed out of order
/// (behind an already-delivered higher id) is picked up here and delivered by the *live* holder
/// without a restart (issue #18 / DECISIONS 0011). The `seen` set deduplicates: only ids newly
/// inserted under its lock are queued, so concurrent poll/push/startup drains never double-queue.
async fn drain(backend: &Arc<dyn Backend>, address: &str, st: &State, source: &str) {
    let rows = match backend.fetch_undelivered(address).await {
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
    let mut seen = st.seen.lock().await;
    let mut q = st.queue.lock().await;
    let mut queued = 0usize;
    for row in rows {
        // `insert` returns false if this holder already queued the id — skip it. This is the only
        // dedup gate; because `seen` is never pruned, a stale fetch cannot resurrect a delivered id.
        if seen.insert(row.id) {
            q.push_back(Buffered {
                row,
                buffered_at_ms: recv,
            });
            queued += 1;
        }
    }
    drop(q);
    let seen_len = seen.len();
    drop(seen);
    if queued > 0 {
        // Log the dedup-set size alongside the queue activity so the monotonic `seen` growth is
        // observable on a pinned, long-lived holder before it could ever matter (see `seen` doc).
        eprintln!("[holder] drain ({source}) queued {queued} (seen={seen_len})");
        st.notify.notify_waiters();
    }
}

pub async fn run(ctx: &Ctx, args: AttachArgs) -> Result<i32> {
    let address = ctx.cfg.require_address(&ctx.address)?;
    let backend = ctx.backend().await?;

    let pid = std::process::id() as i64;
    let occupant = args
        .occupant
        .clone()
        .unwrap_or_else(|| format!("{}:{}", config::hostname(), pid));

    backend
        .ensure_address(
            &address,
            args.description.as_deref(),
            args.scope.as_deref(),
            args.tags.as_deref(),
        )
        .await?;

    let claim = LeaseClaim {
        address: address.clone(),
        occupant: occupant.clone(),
        host: config::hostname(),
        principal: config::principal(),
        description: args.description.clone(),
        tags: args.tags.clone(),
        scope: args.scope.clone(),
        pid,
    };
    match backend
        .claim_lease(&claim, ctx.cfg.liveness_window_secs)
        .await?
    {
        LeaseOutcome::Claimed => {}
        LeaseOutcome::AlreadyOccupied(lease) => {
            let out = serde_json::json!({
                "error": "address-occupied",
                "address": address,
                "occupant": lease.occupant,
                "host": lease.host,
                "principal": lease.principal,
            });
            emit(ctx.fmt, &out, || {
                eprintln!(
                    "telex: address {address} already occupied by {} on {}",
                    lease.occupant.as_deref().unwrap_or("?"),
                    lease.host.as_deref().unwrap_or("?")
                );
            });
            return Ok(1);
        }
    }

    // Delivery state — not a monotonic id cursor — drives the queue (DECISIONS 0011). The holder
    // seeds and refills its queue from `fetch_undelivered` via the initial drain below (run after
    // the heartbeat task is live), so a Postgres id committed out of order is delivered by the live
    // holder without waiting for a restart.
    let state = Arc::new(State {
        queue: Mutex::new(VecDeque::new()),
        notify: Notify::new(),
        seen: Mutex::new(HashSet::new()),
        last_heartbeat_ms: AtomicI64::new(now_ms()),
        keepalive: Duration::from_secs(args.keepalive_secs.max(1)),
        shutdown: Notify::new(),
        backend: backend.clone(),
        address: address.clone(),
        occupant: occupant.clone(),
    });

    eprintln!(
        "[holder] pid={pid} backend={} address={address} heartbeat={}s poll={}s push={}",
        backend.kind(),
        args.heartbeat_secs,
        args.poll_secs,
        args.push
    );

    // Heartbeat task.
    {
        let st = state.clone();
        let backend = backend.clone();
        let address = address.clone();
        let interval = args.heartbeat_secs.max(1);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(interval));
            loop {
                tick.tick().await;
                match backend.heartbeat(&address).await {
                    Ok(_) => st.last_heartbeat_ms.store(now_ms(), Ordering::SeqCst),
                    Err(e) => eprintln!("[holder] heartbeat error: {e}"),
                }
            }
        });
    }

    // Initial drain now that the heartbeat task is keeping the lease fresh (a large first-upgrade
    // backlog can take a moment to materialize, and we don't want that to delay the first heartbeat
    // and make the freshly-claimed lease look stale within the liveness window). This is the same
    // `drain` the poll/push tasks run: it queues every undelivered, non-terminal message addressed
    // here — including everything queued while the address was unoccupied — and the monotonic `seen`
    // guard keeps it from racing the poll task's first tick into a double-queue.
    drain(&backend, &address, &state, "startup").await;

    // Poll task (delivery backstop for both backends).
    {
        let st = state.clone();
        let backend = backend.clone();
        let address = address.clone();
        let interval = args.poll_secs.max(1);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(interval));
            loop {
                tick.tick().await;
                drain(&backend, &address, &st, "poll").await;
            }
        });
    }

    // Optional Postgres push (no-op where the backend or this build lacks it).
    if args.push {
        #[cfg(feature = "postgres")]
        if backend.kind() == "postgres" {
            let (_n, profile) = ctx.resolved()?;
            let (pgcfg, _schema) = crate::profiles::pg_connect_config(&profile).await?;
            spawn_pg_push(&state, &backend, &address, pgcfg);
        } else {
            eprintln!(
                "[holder] --push ignored: backend {} has no native push",
                backend.kind()
            );
        }
        #[cfg(not(feature = "postgres"))]
        eprintln!("[holder] --push ignored: this build has no postgres backend");
    }

    // Serve waiters until shutdown.
    let mut listener = ipc::Listener::bind(&address)?;
    eprintln!("[holder] listening for waiters on {address}");

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok(conn) => {
                        let st = state.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_conn(conn, st).await {
                                eprintln!("[holder] conn error: {e}");
                            }
                        });
                    }
                    Err(e) => eprintln!("[holder] accept error: {e}"),
                }
            }
            _ = state.shutdown.notified() => {
                eprintln!("[holder] shutdown requested");
                break;
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("[holder] interrupted");
                break;
            }
        }
    }

    let released = backend
        .release_lease(&address, &occupant)
        .await
        .unwrap_or(false);
    eprintln!("[holder] lease released={released}; exiting");
    Ok(0)
}

/// Spawn the optional Postgres LISTEN/NOTIFY push tasks using a ready-to-connect config.
/// Compiled only with the postgres feature; poll remains the delivery backstop regardless.
#[cfg(feature = "postgres")]
fn spawn_pg_push(
    state: &Arc<State>,
    backend: &Arc<dyn Backend>,
    address: &str,
    pg_config: tokio_postgres::Config,
) {
    use crate::backend::postgres::{make_tls, NOTIFY_CHANNEL};
    use futures_util::{stream, StreamExt};
    use tokio_postgres::AsyncMessage;

    let notify_new = Arc::new(Notify::new());

    // LISTEN connection, driven manually so notifications surface.
    {
        let notify_new = notify_new.clone();
        let address = address.to_string();
        tokio::spawn(async move {
            let (listen_client, mut listen_conn) = match async {
                let tls = make_tls()?;
                Ok::<_, anyhow::Error>(pg_config.connect(tls).await?)
            }
            .await
            {
                Ok(pair) => pair,
                Err(e) => {
                    eprintln!("[holder] push listen connect failed: {e}");
                    return;
                }
            };
            let messages = stream::poll_fn(move |cx| listen_conn.poll_message(cx));
            tokio::pin!(messages);
            if let Err(e) = listen_client
                .batch_execute(&format!("LISTEN {NOTIFY_CHANNEL}"))
                .await
            {
                eprintln!("[holder] LISTEN failed: {e}");
                return;
            }
            eprintln!("[holder] push enabled (LISTEN {NOTIFY_CHANNEL})");
            while let Some(msg) = messages.next().await {
                match msg {
                    Ok(AsyncMessage::Notification(note)) => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(note.payload()) {
                            if v.get("address").and_then(|a| a.as_str()) == Some(address.as_str()) {
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
    }

    // React to push signals by draining immediately.
    {
        let st = state.clone();
        let backend = backend.clone();
        let address = address.to_string();
        tokio::spawn(async move {
            loop {
                notify_new.notified().await;
                drain(&backend, &address, &st, "push").await;
            }
        });
    }
}

async fn handle_conn<S>(stream: S, st: Arc<State>) -> Result<()>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(());
    }
    let req: Request = serde_json::from_str(line.trim())?;

    match req {
        Request::Ping => {
            let age = now_ms() - st.last_heartbeat_ms.load(Ordering::SeqCst);
            send(
                &mut write_half,
                &Frame::Pong {
                    heartbeat_age_ms: age,
                },
            )
            .await
        }
        Request::Shutdown => {
            send(&mut write_half, &Frame::ShuttingDown).await?;
            st.shutdown.notify_one();
            Ok(())
        }
        Request::Wait { timeout_ms, .. } => {
            let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
            let mut ka = tokio::time::interval(st.keepalive);
            ka.tick().await; // consume the immediate first tick
            loop {
                if let Some(buf) = {
                    let mut q = st.queue.lock().await;
                    q.pop_front()
                } {
                    let frame = Frame::Message {
                        id: buf.row.id,
                        thread_id: buf.row.thread_id,
                        parent_id: buf.row.parent_id,
                        from_addr: buf.row.from_addr.clone(),
                        to_addr: buf.row.to_addr.clone(),
                        kind: buf.row.kind.clone(),
                        attention: buf.row.attention.clone(),
                        requires_disposition: buf.row.requires_disposition,
                        subject: buf.row.subject.clone(),
                        body: buf.row.body.clone(),
                        sent_at_ms: buf.row.sent_at_ms,
                        buffered_at_ms: buf.buffered_at_ms,
                    };
                    match send(&mut write_half, &frame).await {
                        Ok(()) => {
                            // Record the handoff durably so a later holder won't redeliver this
                            // message. We mark after the frame is on the wire: delivery is committed
                            // at the holder->waiter handoff (at-least-once across restarts). A mark
                            // failure only risks a future duplicate, never a loss, so log and still
                            // report the delivery as successful.
                            let id = buf.row.id;
                            if let Err(e) = st
                                .backend
                                .mark_delivered(id, &st.address, Some(&st.occupant))
                                .await
                            {
                                eprintln!("[holder] mark_delivered failed for id={id}: {e}");
                            }
                            return Ok(());
                        }
                        Err(e) => {
                            // The waiter never received it (write/flush failed). Requeue at the front
                            // so the next waiter still gets it instead of dropping it on a transient
                            // connection error, then nudge any waiter currently parked on `notify`.
                            // `notify_waiters()` stores no permit: if no waiter is parked right now the
                            // wake is a no-op and the message is picked up by the next `wait` at the
                            // loop top (or, for a `--timeout-ms` waiter, the next keepalive re-check).
                            {
                                let mut q = st.queue.lock().await;
                                q.push_front(buf);
                            }
                            st.notify.notify_waiters();
                            return Err(e);
                        }
                    }
                }
                if let Some(deadline) = deadline {
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
                } else {
                    tokio::select! {
                        _ = st.notify.notified() => continue,
                        _ = ka.tick() => {
                            let age = now_ms() - st.last_heartbeat_ms.load(Ordering::SeqCst);
                            send(&mut write_half, &Frame::Keepalive { heartbeat_age_ms: age }).await?;
                        }
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

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    //! Holder delivery-path tests. They drive the real `handle_conn` over an in-memory
    //! `tokio::io::duplex` pair (no OS IPC, no spawned process), so the durable-delivery mark and
    //! the requeue-on-write-failure behavior are exercised deterministically on every platform.
    use super::*;
    use crate::backend::sqlite::SqliteBackend;
    use crate::ipc::{Frame, Request};
    use crate::model::{Attention, NewMessage};
    use tokio::io::{duplex, AsyncBufReadExt, AsyncWriteExt, BufReader};

    async fn temp_backend() -> Arc<dyn Backend> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        // pid + now_ms + a process-wide counter so concurrently-running tests never share a db file
        // (a shared file races into "database is locked").
        let seq = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "telex-attach-test-{}-{}-{}",
            std::process::id(),
            now_ms(),
            seq
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.db").to_string_lossy().to_string();
        let b = SqliteBackend::open(&path).expect("open sqlite");
        b.init_schema().await.expect("init schema");
        Arc::new(b)
    }

    fn note(to: &str) -> NewMessage {
        NewMessage {
            parent_id: None,
            from_addr: None,
            to_addr: to.to_string(),
            cc: None,
            kind: "note".to_string(),
            attention: Attention::Background,
            requires_disposition: false,
            subject: None,
            body: "hi".to_string(),
            metadata: None,
            sent_at_ms: now_ms(),
        }
    }

    fn state_with(backend: Arc<dyn Backend>, address: &str) -> Arc<State> {
        Arc::new(State {
            queue: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
            seen: Mutex::new(HashSet::new()),
            last_heartbeat_ms: AtomicI64::new(now_ms()),
            keepalive: Duration::from_secs(30),
            shutdown: Notify::new(),
            backend,
            address: address.to_string(),
            occupant: "holder:test".to_string(),
        })
    }

    async fn wait_request(addr: &str) -> Vec<u8> {
        let mut line = serde_json::to_string(&Request::Wait {
            address: addr.to_string(),
            since: 0,
            timeout_ms: None,
        })
        .unwrap();
        line.push('\n');
        line.into_bytes()
    }

    /// A successful handoff delivers the queued message AND records it durably, so a later holder
    /// will not redeliver it: `fetch_undelivered` no longer returns it after delivery.
    #[tokio::test]
    async fn delivers_queued_message_and_records_durable_delivery() {
        let backend = temp_backend().await;
        let addr = "attach:deliver";
        let row = backend.insert_message(&note(addr)).await.unwrap();
        assert_eq!(
            backend.fetch_undelivered(addr).await.unwrap().len(),
            1,
            "message is undelivered before it is delivered"
        );

        let st = state_with(backend.clone(), addr);
        st.queue.lock().await.push_back(Buffered {
            row: row.clone(),
            buffered_at_ms: now_ms(),
        });

        let (client, server) = duplex(4096);
        let holder = tokio::spawn({
            let st = st.clone();
            async move { handle_conn(server, st).await }
        });

        let (cr, mut cw) = tokio::io::split(client);
        cw.write_all(&wait_request(addr).await).await.unwrap();
        cw.flush().await.unwrap();

        let mut reader = BufReader::new(cr);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        match serde_json::from_str::<Frame>(line.trim()).unwrap() {
            Frame::Message { id, .. } => assert_eq!(id, row.id, "delivered the queued message"),
            other => panic!("expected a Message frame, got {other:?}"),
        }

        holder
            .await
            .unwrap()
            .expect("handle_conn returns Ok on a clean handoff");

        assert!(
            backend.fetch_undelivered(addr).await.unwrap().is_empty(),
            "a delivered message must not reappear as undelivered"
        );
    }

    /// If the waiter vanishes before the frame is written, the message is requeued (not lost) and
    /// not marked delivered, so a later waiter still receives it.
    #[tokio::test]
    async fn requeues_message_when_waiter_write_fails() {
        use std::pin::Pin;
        use std::task::{Context, Poll};
        use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

        // A holder-side stream that yields a canned request to read, then fails every write —
        // modeling a waiter that disconnected before the holder could deliver the frame. This makes
        // the write failure deterministic, unlike racing a dropped duplex peer.
        struct ReadThenFailWrite {
            to_read: Vec<u8>,
            pos: usize,
        }
        impl AsyncRead for ReadThenFailWrite {
            fn poll_read(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                buf: &mut ReadBuf<'_>,
            ) -> Poll<std::io::Result<()>> {
                let remaining = self.to_read.len() - self.pos;
                let n = remaining.min(buf.remaining());
                if n > 0 {
                    let start = self.pos;
                    buf.put_slice(&self.to_read[start..start + n]);
                    self.pos += n;
                }
                Poll::Ready(Ok(())) // n == 0 signals EOF once the request is drained
            }
        }
        impl AsyncWrite for ReadThenFailWrite {
            fn poll_write(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                _buf: &[u8],
            ) -> Poll<std::io::Result<usize>> {
                Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "waiter gone",
                )))
            }
            fn poll_flush(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::io::Result<()>> {
                Poll::Ready(Ok(()))
            }
            fn poll_shutdown(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::io::Result<()>> {
                Poll::Ready(Ok(()))
            }
        }

        let backend = temp_backend().await;
        let addr = "attach:requeue";
        let row = backend.insert_message(&note(addr)).await.unwrap();

        let st = state_with(backend.clone(), addr);
        st.queue.lock().await.push_back(Buffered {
            row: row.clone(),
            buffered_at_ms: now_ms(),
        });

        let stream = ReadThenFailWrite {
            to_read: wait_request(addr).await,
            pos: 0,
        };
        let result = handle_conn(stream, st.clone()).await;
        assert!(
            result.is_err(),
            "handle_conn should surface the failed frame write"
        );

        assert_eq!(
            st.queue.lock().await.len(),
            1,
            "the message is requeued after a write failure, not dropped"
        );
        assert_eq!(
            backend.fetch_undelivered(addr).await.unwrap().len(),
            1,
            "a message that never reached a waiter stays undelivered (no spurious delivery mark)"
        );
    }

    /// The gap-closing invariant at the holder level: a lower undelivered id is delivered by the
    /// LIVE holder even after a higher id has already been delivered — the consequence of a Postgres
    /// out-of-order commit, reproduced deterministically on SQLite (issue #18). The old high-water
    /// cursor (seeded to the higher id) would have skipped the lower id until a restart; the
    /// delivery-state drain queues it and a waiter actually receives it.
    #[tokio::test]
    async fn live_drain_delivers_lower_id_behind_a_delivered_higher_id() {
        let backend = temp_backend().await;
        let addr = "attach:reorder";
        let lower = backend.insert_message(&note(addr)).await.unwrap();
        let higher = backend.insert_message(&note(addr)).await.unwrap();
        assert!(lower.id < higher.id);

        // The higher id was already handed off (its durable delivery record exists); the lower id
        // never was — exactly the state a commit-order skip leaves behind.
        backend
            .mark_delivered(higher.id, addr, Some("holderA"))
            .await
            .unwrap();

        let st = state_with(backend.clone(), addr);
        // A live drain (poll/push/startup all share this path) must queue the lower id.
        drain(&st.backend, addr, &st, "test").await;
        assert_eq!(
            st.queue.lock().await.iter().map(|b| b.row.id).collect::<Vec<_>>(),
            vec![lower.id],
            "drain queues the undelivered lower id and not the already-delivered higher id"
        );

        // ...and a real waiter receives it over the IPC handoff (the literal acceptance bar).
        let (client, server) = duplex(4096);
        let holder = tokio::spawn({
            let st = st.clone();
            async move { handle_conn(server, st).await }
        });
        let (cr, mut cw) = tokio::io::split(client);
        cw.write_all(&wait_request(addr).await).await.unwrap();
        cw.flush().await.unwrap();
        let mut reader = BufReader::new(cr);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        match serde_json::from_str::<Frame>(line.trim()).unwrap() {
            Frame::Message { id, .. } => {
                assert_eq!(id, lower.id, "the live holder delivered the lower id, no restart")
            }
            other => panic!("expected a Message frame, got {other:?}"),
        }
        holder.await.unwrap().expect("clean handoff");

        // Nothing remains undelivered, and a second drain re-queues nothing (idempotent).
        assert!(backend.fetch_undelivered(addr).await.unwrap().is_empty());
        drain(&st.backend, addr, &st, "test").await;
        assert!(
            st.queue.lock().await.is_empty(),
            "a second drain does not re-queue a delivered id"
        );
    }

    /// `drain` is idempotent within a holder: re-draining an already-queued (but not yet delivered)
    /// message does not duplicate it. This is the monotonic-`seen` guard that makes concurrent
    /// poll/push/startup drains safe (planning-review blocking finding #1).
    #[tokio::test]
    async fn drain_does_not_requeue_an_already_queued_message() {
        let backend = temp_backend().await;
        let addr = "attach:idemp";
        backend.insert_message(&note(addr)).await.unwrap();

        let st = state_with(backend.clone(), addr);
        drain(&st.backend, addr, &st, "first").await;
        drain(&st.backend, addr, &st, "second").await;
        assert_eq!(
            st.queue.lock().await.len(),
            1,
            "the message is queued exactly once across repeated drains"
        );
    }

    /// A message whose latest disposition is terminal is excluded from the live drain (consistent
    /// with the durable-backlog path): an out-of-band `telex handle` before any waiter pops it means
    /// the message is already handled and must not be delivered.
    #[tokio::test]
    async fn drain_excludes_terminally_dispositioned_message() {
        let backend = temp_backend().await;
        let addr = "attach:terminal";
        let row = backend.insert_message(&note(addr)).await.unwrap();
        backend
            .insert_disposition(row.id, addr, "handled", None, None)
            .await
            .unwrap();

        let st = state_with(backend.clone(), addr);
        drain(&st.backend, addr, &st, "test").await;
        assert!(
            st.queue.lock().await.is_empty(),
            "a terminally dispositioned message is not queued by the live drain"
        );
    }

    /// Concurrency safety of the drain dedup (SF-6): startup/poll/push all call `drain` and may run
    /// concurrently; the ONLY thing preventing a double-enqueue is the monotonic `seen` lock. Drive
    /// many drains truly in parallel (multi-thread runtime) over the same `Arc<State>` and assert
    /// each message is queued exactly once. This pins the race so a future lock-narrowing change
    /// (e.g. releasing `seen` across the awaited fetch) that re-introduced the TOCTOU would fail here.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_drains_never_double_enqueue() {
        let backend = temp_backend().await;
        let addr = "attach:concurrent";
        const N: usize = 25;
        for _ in 0..N {
            backend.insert_message(&note(addr)).await.unwrap();
        }

        let st = state_with(backend.clone(), addr);
        let mut handles = Vec::new();
        for _ in 0..8 {
            let st = st.clone();
            let backend = backend.clone();
            let addr = addr.to_string();
            handles.push(tokio::spawn(async move {
                drain(&backend, &addr, &st, "race").await;
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let queued: Vec<i64> = st.queue.lock().await.iter().map(|b| b.row.id).collect();
        let distinct: HashSet<i64> = queued.iter().copied().collect();
        assert_eq!(
            queued.len(),
            N,
            "exactly N messages queued — concurrent drains never double-enqueue"
        );
        assert_eq!(distinct.len(), N, "every queued id is distinct");
    }

    /// A message terminally dispositioned out-of-band AFTER it is queued is still delivered (TO-2):
    /// `handle_conn` does not re-check disposition at the handoff. Pinning this guards the no-drop
    /// invariant — a future maintainer must not add a naive pop-time disposition re-check that could
    /// drop an already-buffered message (the live drain's pre-queue filter is the only exclusion).
    #[tokio::test]
    async fn terminal_disposition_after_queue_still_delivers() {
        let backend = temp_backend().await;
        let addr = "attach:terminal-after-queue";
        let row = backend.insert_message(&note(addr)).await.unwrap();

        let st = state_with(backend.clone(), addr);
        drain(&st.backend, addr, &st, "test").await;
        assert_eq!(st.queue.lock().await.len(), 1, "message queued");

        // Out-of-band terminal disposition AFTER queueing (e.g. via `telex inbox`).
        backend
            .insert_disposition(row.id, addr, "handled", None, None)
            .await
            .unwrap();

        let (client, server) = duplex(4096);
        let holder = tokio::spawn({
            let st = st.clone();
            async move { handle_conn(server, st).await }
        });
        let (cr, mut cw) = tokio::io::split(client);
        cw.write_all(&wait_request(addr).await).await.unwrap();
        cw.flush().await.unwrap();
        let mut reader = BufReader::new(cr);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        match serde_json::from_str::<Frame>(line.trim()).unwrap() {
            Frame::Message { id, .. } => assert_eq!(
                id, row.id,
                "an already-queued message is delivered despite a later terminal disposition"
            ),
            other => panic!("expected a Message frame, got {other:?}"),
        }
        holder.await.unwrap().expect("clean handoff");
    }
}
