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
    queue: Mutex<VecDeque<Buffered>>,
    notify: Notify,
    cursor: Mutex<i64>,
    last_heartbeat_ms: AtomicI64,
    keepalive: Duration,
    shutdown: Notify,
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

    let state = Arc::new(State {
        queue: Mutex::new(VecDeque::new()),
        notify: Notify::new(),
        cursor: Mutex::new(backend.max_id(&address).await?),
        last_heartbeat_ms: AtomicI64::new(now_ms()),
        keepalive: Duration::from_secs(args.keepalive_secs.max(1)),
        shutdown: Notify::new(),
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
                    let m = buf.row;
                    return send(
                        &mut write_half,
                        &Frame::Message {
                            id: m.id,
                            thread_id: m.thread_id,
                            parent_id: m.parent_id,
                            from_addr: m.from_addr,
                            to_addr: m.to_addr,
                            kind: m.kind,
                            attention: m.attention,
                            requires_disposition: m.requires_disposition,
                            subject: m.subject,
                            body: m.body,
                            sent_at_ms: m.sent_at_ms,
                            buffered_at_ms: buf.buffered_at_ms,
                        },
                    )
                    .await;
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
