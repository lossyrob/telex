//! `telex attach`: the resident holder and answerback drum. Claims an exclusive lease,
//! writes a TTL heartbeat, polls the backend for new messages (optionally adds Postgres
//! LISTEN/NOTIFY push), buffers them, and serves ephemeral `wait` clients over local IPC.
//! Blocks for the mission; releases the lease on shutdown (Ctrl-C or a `detach` signal).

use anyhow::Result;
use std::collections::VecDeque;
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
    address: String,
    backend_key: String,
    queue: Mutex<VecDeque<Buffered>>,
    notify: Notify,
    cursor: Mutex<i64>,
    last_heartbeat_ms: AtomicI64,
    keepalive: Duration,
    shutdown: Notify,
    backend: Arc<dyn Backend>,
    occupant: String,
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
    for row in rows {
        *cur = row.id;
        q.push_back(Buffered {
            row,
            buffered_at_ms: recv,
        });
    }
    drop(q);
    drop(cur);
    st.notify.notify_waiters();
}

pub async fn run(ctx: &Ctx, args: AttachArgs) -> Result<i32> {
    let address = ctx.cfg.require_address(&ctx.address)?;
    let backend = ctx.backend().await?;

    let pid = std::process::id() as i64;
    let backend_key = ctx.store_key()?;
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

    // `start_cursor` is the delivery high-water for the streaming drain. The holder seeds its queue
    // from the durable backlog further below — after the heartbeat task is live — so a restart
    // recovers the messages queued while the address was unoccupied instead of skipping past
    // `max_id`.
    let start_cursor = backend.max_id(&address).await?;

    // Record this station in the session ownership registry so a Copilot CLI `sessionEnd` hook can
    // detach it when the session ends (dismiss or quit). Best-effort: never fail attach on this.
    if let Err(e) = crate::session_registry::register_station(&address) {
        eprintln!("[holder] session registry: register failed (continuing): {e}");
    }

    let state = Arc::new(State {
        address: address.clone(),
        backend_key: backend_key.clone(),
        queue: Mutex::new(VecDeque::new()),
        notify: Notify::new(),
        cursor: Mutex::new(start_cursor),
        last_heartbeat_ms: AtomicI64::new(now_ms()),
        keepalive: Duration::from_secs(args.keepalive_secs.max(1)),
        shutdown: Notify::new(),
        backend: backend.clone(),
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
            // After a host sleep/stall, skip missed ticks rather than bursting catch-up work.
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                match backend.heartbeat(&address).await {
                    Ok(_) => st.last_heartbeat_ms.store(now_ms(), Ordering::SeqCst),
                    Err(e) => eprintln!("[holder] heartbeat error: {e}"),
                }
            }
        });
    }

    // Recover the durable backlog now that the heartbeat task is keeping the lease fresh — a large
    // first-upgrade backlog can take a moment to materialize, and we don't want that to delay the
    // first heartbeat and make the freshly-claimed lease look stale within the liveness window. The
    // backlog is everything at or below `start_cursor` that was queued (and not yet delivered or
    // terminally dispositioned) while the address was unoccupied; the `id <= start_cursor` bound
    // makes the seeded backlog and the `fetch_after` (id > cursor) drain partition cleanly, so a
    // message inserted between the two snapshots is drained — never both drained and seeded. This is
    // a start-time snapshot: a message terminally dispositioned via `telex inbox` after seeding but
    // before a waiter pops it is still delivered once here (and then marked, so it is not
    // re-recovered on a later restart).
    let backlog = backend.undelivered_backlog(&address, start_cursor).await?;
    if !backlog.is_empty() {
        let recv = now_ms();
        let n = backlog.len();
        let mut q = state.queue.lock().await;
        for row in backlog {
            q.push_back(Buffered {
                row,
                buffered_at_ms: recv,
            });
        }
        drop(q);
        eprintln!("[holder] recovered {n} queued message(s) from durable backlog");
    }

    // Poll task (delivery backstop for both backends).
    {
        let st = state.clone();
        let backend = backend.clone();
        let address = address.clone();
        let interval = args.poll_secs.max(1);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(interval));
            // After a host sleep/stall, skip missed ticks rather than bursting catch-up polls.
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                drain_new(&backend, &address, &st, "poll").await;
            }
        });
    }

    // Session-binding watch (issue #5). When the holder is bound to a launcher/session pid, poll
    // it and, the moment it is gone, route through the *same* shutdown path as `detach`/ctrl-c so
    // the lease is released identically. Defense-in-depth: even a mis-launched detached holder
    // cannot outlive the session that spawned it. The env var is read here (not by clap) so a
    // malformed `$TELEX_SESSION_PID` never fails `--no-session-bind`.
    use crate::session_watch::{SessionBinding, UnboundReason};
    let env_pid = std::env::var("TELEX_SESSION_PID").ok();
    match crate::session_watch::resolve_session_pid(
        args.no_session_bind,
        args.session_pid,
        env_pid.as_deref(),
    ) {
        SessionBinding::Bound(session_pid) => {
            // Keep the liveness check inside the lease window so the address always frees within
            // it, even if a caller passes a poll interval larger than the window.
            let window = ctx.cfg.liveness_window_secs.max(1) as u64;
            let interval = args.session_poll_secs.max(1).min(window);
            eprintln!(
                "[holder] session-bound to pid {session_pid} (releases lease if it exits; poll {interval}s)"
            );
            let st = state.clone();
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(interval));
                // A liveness probe gains nothing from catch-up ticks: after a host sleep/stall,
                // skip the missed ticks instead of bursting a run of `process_alive` syscalls.
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tick.tick().await;
                    if !crate::session_watch::process_alive(session_pid) {
                        eprintln!(
                            "[holder] session {session_pid} gone; releasing lease and exiting"
                        );
                        st.shutdown.notify_one();
                        break;
                    }
                }
            });
        }
        // Always log *why* binding is off so the holder's binding state is visible in an incident
        // (the silent legacy default is the only quiet case).
        SessionBinding::Unbound(reason) => match reason {
            UnboundReason::NotRequested => {}
            UnboundReason::OptedOut => {
                if args.session_pid.is_some() || env_pid.is_some() {
                    eprintln!(
                        "[holder] --no-session-bind set; ignoring any session pid and running persistent"
                    );
                }
            }
            UnboundReason::ZeroSentinel => {
                eprintln!("[holder] session pid 0 (unbound sentinel); running persistent");
            }
            UnboundReason::MalformedEnv(raw) => {
                eprintln!(
                    "[holder] TELEX_SESSION_PID={raw:?} is not a valid pid; running persistent (unbound)"
                );
            }
        },
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

    // Publish the local holder registry record now that the endpoint is live, so `send`/`reply`
    // can default `from` to this lease. We hold the exclusive lease, so any prior record for this
    // (address, backend) is stale — prune it first. Best-effort: a failure here only disables the
    // from-default convenience, never the station itself.
    crate::registry::prune_address(&address, &backend_key);
    let record = crate::registry::HolderRecord {
        address: address.clone(),
        backend: backend_key.clone(),
        host: config::hostname(),
        pid,
        socket: ipc::endpoint(&address),
        started_at_ms: now_ms(),
    };
    if let Err(e) = crate::registry::write(&record) {
        eprintln!("[holder] registry write failed (from-default disabled): {e}");
    }

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

    crate::registry::remove(&address, pid);
    let released = backend
        .release_lease(&address, &occupant)
        .await
        .unwrap_or(false);
    // Drop this station from the session ownership registry; the lease is gone, so the sessionEnd
    // hook should no longer try to detach it. Best-effort.
    if let Err(e) = crate::session_registry::unregister_station(&address) {
        eprintln!("[holder] session registry: unregister failed (ignoring): {e}");
    }
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
                drain_new(&backend, &address, &st, "push").await;
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
                    served_address: Some(st.address.clone()),
                    served_backend: Some(st.backend_key.clone()),
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
            backend_key: backend.kind().to_string(),
            queue: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
            cursor: Mutex::new(0),
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
    /// will not redeliver it: `undelivered_backlog` no longer returns it after delivery.
    #[tokio::test]
    async fn delivers_queued_message_and_records_durable_delivery() {
        let backend = temp_backend().await;
        let addr = "attach:deliver";
        let row = backend.insert_message(&note(addr)).await.unwrap();
        let head = backend.max_id(addr).await.unwrap();
        assert_eq!(
            backend.undelivered_backlog(addr, head).await.unwrap().len(),
            1,
            "message is in the backlog before it is delivered"
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
            backend
                .undelivered_backlog(addr, head)
                .await
                .unwrap()
                .is_empty(),
            "a delivered message must not reappear in the restart backlog"
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
        let head = backend.max_id(addr).await.unwrap();
        assert_eq!(
            backend.undelivered_backlog(addr, head).await.unwrap().len(),
            1,
            "a message that never reached a waiter stays in the backlog (no spurious delivery mark)"
        );
    }
}
