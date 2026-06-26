#![cfg(feature = "sqlite")]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use telex::backend::sqlite::SqliteBackend;
use telex::backend::Backend;
use telex::daemon::test_support::{self, registered_epoch, TestClientAction, TestDaemon};
use telex::daemon::{verify_admin_proof, DaemonPaths, SingletonKey};
use telex::daemon_ipc::{
    self as proto, send_hello_after_verifier, Request, Response, WatchPidRole, WatchPidSpec,
};
use telex::model::{now_ms, Attention, DeliveryOutcome, EpochClaimResult, NewMessage};
use tokio::io::{AsyncReadExt, AsyncWrite};
use tokio::sync::Barrier;

fn assert_needs_attach(response: Response) {
    assert!(
        matches!(response, Response::Error { ref code, .. } if code == proto::ERROR_NEEDS_ATTACH),
        "expected NeedsAttach, got {response:?}"
    );
}

fn assert_error_code(response: Response, expected: &str) {
    assert!(
        matches!(response, Response::Error { ref code, .. } if code == expected),
        "expected {expected}, got {response:?}"
    );
}

fn assert_registered(response: Response) -> (i64, String) {
    match response {
        Response::Registered {
            lease_epoch,
            owner_instance_id,
        } => (lease_epoch, owner_instance_id),
        other => panic!("expected Registered, got {other:?}"),
    }
}

fn assert_ack_outcome(response: Response, expected: DeliveryOutcome) {
    match response {
        Response::Ack {
            delivery_outcome, ..
        } => assert_eq!(delivery_outcome, Some(expected)),
        other => panic!("expected Ack, got {other:?}"),
    }
}

async fn insert_message(backend: &Arc<dyn Backend>, to: &str, cc: Option<&str>) -> i64 {
    backend
        .insert_message(&NewMessage {
            parent_id: None,
            from_addr: Some("sender".to_string()),
            to_addr: to.to_string(),
            cc: cc.map(str::to_string),
            kind: "note".to_string(),
            attention: Attention::Background,
            requires_disposition: false,
            subject: None,
            body: "hello".to_string(),
            metadata: None,
            sent_at_ms: now_ms(),
        })
        .await
        .expect("insert message")
        .id
}

async fn rotate_owner(
    backend: &Arc<dyn Backend>,
    address: &str,
    predecessor: &str,
    predecessor_epoch: i64,
    successor: &str,
) -> i64 {
    assert!(
        backend
            .release_epoch_lease(address, predecessor, predecessor_epoch)
            .await
            .expect("release predecessor"),
        "predecessor should release before successor claim"
    );
    match backend
        .claim_epoch_lease(address, successor, 15)
        .await
        .expect("successor claim")
    {
        EpochClaimResult::Claimed(claimed) => claimed.lease_epoch,
        other => panic!("expected successor claim, got {other:?}"),
    }
}

fn sqlite_path(store_key: &str) -> PathBuf {
    test_support::store_path_from_key(store_key).expect("sqlite store key")
}

fn delivery_counts(store_key: &str) -> (i64, i64) {
    let conn = rusqlite::Connection::open(sqlite_path(store_key)).expect("open sqlite counts");
    let total = conn
        .query_row("SELECT COUNT(*) FROM deliveries", [], |r| {
            r.get::<_, i64>(0)
        })
        .expect("count deliveries");
    let pending = conn
        .query_row(
            "SELECT COUNT(*) FROM deliveries WHERE consumed_at_ms IS NULL",
            [],
            |r| r.get::<_, i64>(0),
        )
        .expect("count pending deliveries");
    (total, pending)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn section17_01_concurrent_first_use() {
    let daemon = TestDaemon::new("section17-01");
    let paths = daemon.paths().clone();
    let barrier = Arc::new(Barrier::new(8));
    let handles = (0..8)
        .map(|_| {
            let paths = paths.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                test_support::bind_listener(&paths).map_err(|e| e.to_string())
            })
        })
        .collect::<Vec<_>>();

    let mut successes = Vec::new();
    let mut failures = Vec::new();
    for handle in handles {
        match handle.await.expect("bind task") {
            Ok(guard) => successes.push(guard),
            Err(err) => failures.push(err),
        }
    }

    assert_eq!(successes.len(), 1, "exactly one singleton listener binds");
    assert_eq!(failures.len(), 7, "all thundering-herd losers fail closed");
    assert!(
        failures.iter().all(|e| {
            e.contains("already")
                || e.contains("live")
                || e.contains("exists")
                || e.contains("busy")
        }),
        "unexpected exclusivity errors: {failures:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn section17_01_concurrent_register_fresh_same_store_shares_backend() {
    let daemon = TestDaemon::new("section17-01-register");
    let store = daemon.store_key("fresh-shared");
    let barrier = Arc::new(Barrier::new(8));
    let tasks = (0..8)
        .map(|i| {
            let worker = daemon.clone();
            let worker_store = store.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                barrier.wait().await;
                worker
                    .register(&worker_store, &format!("s{i}"), &format!("addr:{i}"))
                    .await
            })
        })
        .collect::<Vec<_>>();

    for task in tasks {
        assert_registered(task.await.expect("register task"));
    }
    let status = daemon.status().await;
    assert_eq!(status.stores.len(), 1);
    assert_eq!(status.members.len(), 8);
}

#[tokio::test]
async fn section17_02_singleton_and_store_lock() {
    let daemon = TestDaemon::new("section17-02");
    let paths = daemon.paths().clone();
    let first_endpoint = test_support::bind_listener(&paths).expect("first endpoint bind");
    let second_endpoint = match test_support::bind_listener(&paths) {
        Ok(_) => panic!("second endpoint should be refused"),
        Err(err) => err,
    };
    assert!(
        second_endpoint.to_string().contains("already")
            || second_endpoint.to_string().contains("live")
            || second_endpoint.to_string().contains("exists")
            || second_endpoint.to_string().contains("busy"),
        "unexpected endpoint refusal: {second_endpoint}"
    );
    drop(first_endpoint);

    let db = daemon.store_path("store-lock");
    std::fs::write(&db, []).expect("seed sqlite file");
    let alias = daemon.root().join("stores").join("store-lock-alias.db");
    let _ = std::fs::remove_file(&alias);
    std::fs::hard_link(&db, &alias).expect("hard link alias to same physical store");

    let locked = SqliteBackend::open_locked(&db.to_string_lossy()).expect("first store lock");
    locked.init_schema().await.expect("init locked store");

    let raw = rusqlite::Connection::open(&db).expect("raw sqlite probe");
    raw.execute_batch(
        "BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS lock_probe(id INTEGER); COMMIT;",
    )
    .expect("store advisory lock must not collide with SQLite write locks");

    let second = SqliteBackend::open_locked(&alias.to_string_lossy());
    assert!(
        second.is_err(),
        "alias path to the same physical SQLite file must fail the canonical store lock"
    );
}

#[tokio::test]
async fn section17_03_crash_wait_needs_attach_reregister() {
    let daemon = TestDaemon::new("section17-03");
    let store = daemon.store_key("crash-wait");

    assert_needs_attach(daemon.wait(&store, "s1", "addr:a", 1).await);
    let (epoch, _) = assert_registered(daemon.register(&store, "s1", "addr:a").await);
    assert!(epoch > 0);
    assert!(matches!(
        daemon.wait(&store, "s1", "addr:a", 1).await,
        Response::Timeout
    ));
}

#[tokio::test]
async fn section17_04_restart_no_loss_no_resurrection() {
    let harness = TestDaemon::new("section17-04-paths");
    let store = harness.store_key("restart");
    let path = sqlite_path(&store);
    {
        let backend = SqliteBackend::open(&path.to_string_lossy()).expect("seed sqlite");
        backend.init_schema().await.expect("init seed sqlite");
        insert_message(&(Arc::new(backend) as Arc<dyn Backend>), "addr:a", None).await;
    }

    let restarted = TestDaemon::new("section17-04-restart");
    assert!(restarted.status().await.members.is_empty());
    assert_needs_attach(restarted.wait(&store, "s1", "addr:a", 1).await);
    assert_registered(restarted.register(&store, "s1", "addr:a").await);
    let delivered = restarted.wait(&store, "s1", "addr:a", 1_000).await;
    let message_id = match delivered {
        Response::Message { id, .. } => id,
        other => panic!("expected durable unconsumed message, got {other:?}"),
    };
    assert_ack_outcome(
        restarted.ack(&store, "s1", "addr:a", message_id).await,
        DeliveryOutcome::Marked,
    );
    let (drain, action) = restarted.drain().await;
    assert!(matches!(drain, Response::Ack { .. }));
    assert_eq!(action, TestClientAction::Drain);
    drop(restarted);

    let after_ack_restart = TestDaemon::new("section17-04-after-ack");
    assert_registered(after_ack_restart.register(&store, "s1", "addr:a").await);
    assert!(matches!(
        after_ack_restart.wait(&store, "s1", "addr:a", 1).await,
        Response::Timeout
    ));
    assert_eq!(delivery_counts(&store).0, 1, "consumed row is retained");
}

#[tokio::test]
async fn section17_05_explicit_ack_fanout_dedup() {
    let daemon = TestDaemon::new("section17-05");
    let store = daemon.store_key("ack-fanout");
    registered_epoch(&daemon, &store, "s1", "addr:a").await;
    registered_epoch(&daemon, &store, "s1", "addr:b").await;
    let backend = daemon.backend(&store).await.expect("backend");
    let message_id = insert_message(&backend, "addr:a", Some("addr:b")).await;

    for _ in 0..2 {
        assert!(matches!(
            daemon.wait(&store, "s1", "addr:a", 1_000).await,
            Response::Message { id, .. } if id == message_id
        ));
    }

    assert_ack_outcome(
        daemon.ack(&store, "s1", "addr:a", message_id).await,
        DeliveryOutcome::Marked,
    );
    assert_ack_outcome(
        daemon.ack(&store, "s1", "addr:a", message_id).await,
        DeliveryOutcome::AlreadyConsumed,
    );
    assert!(
        backend
            .fetch_undelivered("addr:a")
            .await
            .expect("fetch a")
            .is_empty(),
        "ack consumes only addr:a"
    );
    assert_eq!(
        backend
            .fetch_undelivered("addr:b")
            .await
            .expect("fetch b")
            .iter()
            .map(|m| m.id)
            .collect::<Vec<_>>(),
        Vec::<i64>::new(),
        "cc fan-out recipient is visible but not wait-deliverable"
    );
    let inbox_b = backend.inbox("addr:b", true, 10).await.expect("inbox b");
    assert!(inbox_b.iter().any(|item| {
        item.message.id == message_id && item.delivery_role == "cc" && !item.actionable
    }));
    assert_needs_attach(daemon.ack(&store, "s1", "addr:c", message_id).await);

    let before_noop = backend.delivery_retention_count().await.expect("retention");
    assert_ack_outcome(
        daemon
            .ack(&store, "s1", "addr:b", message_id + 99_999)
            .await,
        DeliveryOutcome::AckNoOp,
    );
    assert_eq!(
        backend.delivery_retention_count().await.expect("retention"),
        before_noop,
        "AckNoOp inserts no consumed row"
    );
    assert!(matches!(
        daemon.wait(&store, "s1", "addr:b", 1_000).await,
        Response::Timeout
    ));
    assert_ack_outcome(
        daemon.ack(&store, "s1", "addr:b", message_id).await,
        DeliveryOutcome::AlreadyConsumed,
    );
}

#[tokio::test]
async fn section17_05_legacy_no_delivery_row_wait_ack_no_redelivery() {
    let daemon = TestDaemon::new("section17-05-legacy");
    let store = daemon.store_key("legacy-no-delivery");
    let path = sqlite_path(&store);
    {
        let conn = rusqlite::Connection::open(&path).expect("seed legacy sqlite");
        conn.execute_batch(
            "CREATE TABLE messages (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                thread_id     INTEGER,
                parent_id     INTEGER,
                from_addr     TEXT,
                to_addr       TEXT NOT NULL,
                cc            TEXT,
                kind          TEXT NOT NULL DEFAULT 'note',
                attention     TEXT NOT NULL DEFAULT 'background',
                requires_disposition INTEGER NOT NULL DEFAULT 0,
                subject       TEXT,
                body          TEXT NOT NULL,
                metadata      TEXT,
                sent_at_ms    INTEGER NOT NULL,
                created_at_ms INTEGER NOT NULL
            );
            INSERT INTO messages(id, thread_id, from_addr, to_addr, kind, attention, body, sent_at_ms, created_at_ms)
            VALUES (1, 1, 'sender', 'addr:legacy', 'note', 'background', 'legacy', 10, 10);",
        )
        .expect("seed old message");
    }

    assert_registered(daemon.register(&store, "s1", "addr:legacy").await);
    assert!(matches!(
        daemon.wait(&store, "s1", "addr:legacy", 1_000).await,
        Response::Message { id: 1, .. }
    ));
    assert_ack_outcome(
        daemon.ack(&store, "s1", "addr:legacy", 1).await,
        DeliveryOutcome::Marked,
    );
    assert!(matches!(
        daemon.wait(&store, "s1", "addr:legacy", 1).await,
        Response::Timeout
    ));
}

#[tokio::test]
async fn section17_06_sqlite_single_writer_postgres_na() {
    let daemon = TestDaemon::new("section17-06");
    let db = daemon.store_path("single-writer");
    let first = SqliteBackend::open_locked(&db.to_string_lossy()).expect("first writer");
    first.init_schema().await.expect("init first writer");
    let second = SqliteBackend::open_locked(&db.to_string_lossy());
    assert!(
        second.is_err(),
        "SQLite row 6 is Postgres N/A because the canonical store lock enforces one writer"
    );
}

#[tokio::test]
async fn section17_07_ownership_rotation_race() {
    let daemon = TestDaemon::new("section17-07");
    let store = daemon.store_key("owner-rotation");
    let (epoch, _) = registered_epoch(&daemon, &store, "s1", "addr:a").await;
    let backend = daemon.backend(&store).await.expect("backend");
    let message_id = insert_message(&backend, "addr:a", None).await;
    assert!(matches!(
        daemon.wait(&store, "s1", "addr:a", 1_000).await,
        Response::Message { id, .. } if id == message_id
    ));

    let successor_epoch =
        rotate_owner(&backend, "addr:a", daemon.instance_id(), epoch, "successor").await;
    assert_eq!(successor_epoch, epoch + 1);
    assert_eq!(
        backend
            .mark_consumed_if_current_owner("addr:a", "successor", successor_epoch, message_id)
            .await
            .expect("successor mark"),
        DeliveryOutcome::Marked
    );
    assert_ack_outcome(
        daemon.ack(&store, "s1", "addr:a", message_id).await,
        DeliveryOutcome::NotOwner,
    );
    assert!(daemon.status().await.members.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn section17_08_session_end_reaping() {
    let daemon = TestDaemon::new("section17-08");
    let store = daemon.store_key("session-end");
    registered_epoch(&daemon, &store, "s1", "addr:a").await;

    let waiter_daemon = daemon.clone();
    let waiter_store = store.clone();
    let waiter = tokio::spawn(async move {
        waiter_daemon
            .wait(&waiter_store, "s1", "addr:a", 5_000)
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(matches!(
        daemon.session_end(&store, "s1").await,
        Response::Ack { .. }
    ));
    assert!(matches!(
        waiter.await.expect("waiter"),
        Response::PresenceEnded
    ));

    let status = daemon.status().await;
    assert_eq!(status.members.len(), 1);
    assert!(status.members[0].idle);
    assert!(status.recent_errors.iter().any(|e| e.kind == "SessionEnd"));

    let backend = daemon.backend(&store).await.expect("backend");
    let message_id = insert_message(&backend, "addr:a", None).await;
    assert_registered(daemon.register(&store, "s1", "addr:a").await);
    assert!(matches!(
        daemon.wait(&store, "s1", "addr:a", 1_000).await,
        Response::Message { id, .. } if id == message_id
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn section17_09_loader_pid_death() {
    let daemon = TestDaemon::new("section17-09");
    let store = daemon.store_key("watch-pid");
    let watch = WatchPidSpec {
        pid: std::process::id(),
        role: WatchPidRole::Anchor,
    };
    assert_registered(
        daemon
            .register_with_watch_pids(&store, "s1", "addr:a", vec![watch])
            .await,
    );

    let start_time_guard_tested = daemon.skew_first_watch_pid_start_time(&store, "s1", "addr:a");
    if !start_time_guard_tested {
        assert_registered(
            daemon
                .register_with_watch_pids(
                    &store,
                    "s1",
                    "addr:a",
                    vec![WatchPidSpec {
                        pid: u32::MAX,
                        role: WatchPidRole::Anchor,
                    }],
                )
                .await,
        );
    }

    let waiter_daemon = daemon.clone();
    let waiter_store = store.clone();
    let waiter = tokio::spawn(async move {
        waiter_daemon
            .wait(&waiter_store, "s1", "addr:a", 5_000)
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    daemon.heartbeat_once().await;
    assert!(matches!(
        waiter.await.expect("waiter"),
        Response::PresenceEnded
    ));
    let status = daemon.status().await;
    assert_eq!(status.members.len(), 1);
    assert!(status.members[0].idle);
    assert!(status
        .recent_errors
        .iter()
        .any(|e| e.kind == "WatchPidDeath"));
}

#[tokio::test]
async fn section17_10_idle_ttl_presence_ended() {
    let daemon = TestDaemon::new("section17-10");
    let store = daemon.store_key("idle-ttl");
    registered_epoch(&daemon, &store, "s1", "addr:a").await;

    assert!(matches!(
        daemon
            .wait_with_idle_ttl(&store, "s1", "addr:a", 5_000, Duration::from_millis(20))
            .await,
        Response::PresenceEnded
    ));
    let status = daemon.status().await;
    assert_eq!(status.members.len(), 1);
    assert!(status.members[0].idle);
    assert!(status.recent_errors.iter().any(|e| e.kind == "IdleTtlReap"));

    let backend = daemon.backend(&store).await.expect("backend");
    let message_id = insert_message(&backend, "addr:a", None).await;
    assert_registered(daemon.register(&store, "s1", "addr:a").await);
    assert!(matches!(
        daemon.wait(&store, "s1", "addr:a", 1_000).await,
        Response::Message { id, .. } if id == message_id
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn section17_11_reset_non_destructive_audit() {
    let daemon = TestDaemon::new("section17-11");
    let store = daemon.store_key("reset");
    let (epoch, _) = registered_epoch(&daemon, &store, "s1", "addr:a").await;
    let backend = daemon.backend(&store).await.expect("backend");
    let message_id = insert_message(&backend, "addr:a", None).await;

    let waiter_daemon = daemon.clone();
    let waiter_store = store.clone();
    let waiter = tokio::spawn(async move {
        waiter_daemon
            .request(Request::Wait {
                store_key: waiter_store,
                session_id: "s1".to_string(),
                address: "addr:a".to_string(),
                attention: Some("interrupt".to_string()),
                min_attention: None,
                timeout_ms: Some(5_000),
                waiter_pid: Some(std::process::id()),
                waiter_start_time: telex::session_watch::capture_process_start_time(
                    std::process::id(),
                ),
            })
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(matches!(
        daemon.reset(&store, "addr:a").await,
        Response::Ack { .. }
    ));
    assert!(matches!(
        waiter.await.expect("waiter"),
        Response::PresenceEnded
    ));

    let status = daemon.status().await;
    assert_eq!(status.members.len(), 1);
    assert!(status.members[0].idle);
    assert!(status
        .recent_errors
        .iter()
        .any(|e| e.kind == "Reset" && e.message.contains("prior_occupant=occupant-s1")));
    let lease = backend
        .get_lease("addr:a")
        .await
        .expect("lease")
        .expect("lease exists");
    assert_eq!(lease.lease_epoch, Some(epoch));
    assert!(matches!(
        daemon.register(&store, "s1", "addr:a").await,
        Response::Registered { .. }
    ));
    assert!(matches!(
        daemon.wait(&store, "s1", "addr:a", 1_000).await,
        Response::Message { id, .. } if id == message_id
    ));
}

#[tokio::test]
async fn section17_12_epoch_monotonicity() {
    let store;
    let epoch;
    {
        let daemon = TestDaemon::new("section17-12-one");
        store = daemon.store_key("epoch");
        (epoch, _) = registered_epoch(&daemon, &store, "s1", "addr:a").await;
        let (drain, _) = daemon.drain().await;
        assert!(matches!(drain, Response::Ack { .. }));
    }

    let successor = TestDaemon::new("section17-12-two");
    let (next_epoch, _) = registered_epoch(&successor, &store, "s2", "addr:a").await;
    assert_eq!(next_epoch, epoch + 1);
    let backend = successor.backend(&store).await.expect("backend");
    let lease = backend
        .get_lease("addr:a")
        .await
        .expect("lease")
        .expect("lease row retained");
    assert_eq!(lease.lease_epoch, Some(next_epoch));
}

#[tokio::test]
async fn section17_13_ordered_handoff_sqlite_floor() {
    let store;
    let epoch;
    {
        let predecessor = TestDaemon::new("section17-13-pred");
        store = predecessor.store_key("handoff");
        (epoch, _) = registered_epoch(&predecessor, &store, "s1", "addr:a").await;

        let live_successor = TestDaemon::new("section17-13-live-succ");
        assert_error_code(
            live_successor.register(&store, "s2", "addr:a").await,
            proto::ERROR_UNSUPPORTED,
        );

        let (drain, action) = predecessor.drain().await;
        assert!(matches!(drain, Response::Ack { .. }));
        assert_eq!(action, TestClientAction::Drain);
        let backend = predecessor.backend(&store).await.expect("backend");
        let lease = backend
            .get_lease("addr:a")
            .await
            .expect("lease")
            .expect("lease exists");
        assert_eq!(lease.lease_epoch, Some(epoch));
        assert_eq!(lease.owner_instance_id, None);
    }

    let successor = TestDaemon::new("section17-13-succ");
    let (next_epoch, _) = registered_epoch(&successor, &store, "s2", "addr:a").await;
    assert_eq!(next_epoch, epoch + 1);
}

#[tokio::test]
async fn section17_14_os_trust_negatives() {
    let daemon = TestDaemon::new("section17-14");
    let paths = daemon.paths().clone();
    let first_endpoint = test_support::bind_listener(&paths).expect("first endpoint bind");
    assert!(test_support::bind_listener(&paths).is_err());
    drop(first_endpoint);

    let invalid = match verify_admin_proof("secret-admin-cap", Some("wrong-admin-cap")) {
        Ok(()) => panic!("invalid cap should be rejected"),
        Err(response) => response,
    };
    let invalid_text = match invalid {
        Response::Error { message, .. } => message,
        other => panic!("expected unauthorized error, got {other:?}"),
    };
    assert!(!invalid_text.contains("secret-admin-cap"));
    assert!(!invalid_text.contains("wrong-admin-cap"));
    assert!(invalid_text.contains(proto::REDACTED_SECRET));

    // Same-OS-principal CI cannot exercise the different-user squat negative. The local
    // fail-closed surface is client/server authentication before Hello plus redaction.
    let (client, mut server) = tokio::io::duplex(4096);
    let verifier_ran = Arc::new(AtomicBool::new(false));
    let mut guarded = GuardedWriter {
        inner: client,
        verifier_ran: verifier_ran.clone(),
    };
    let hello = proto::client_hello("store");
    let err = send_hello_after_verifier(&mut guarded, &hello, || {
        verifier_ran.store(true, Ordering::SeqCst);
        Err(proto::HandshakeError::Verify(
            "hostile pre-bound endpoint".to_string(),
        ))
    })
    .await
    .expect_err("verifier rejects before Hello");
    assert!(err.to_string().contains("hostile pre-bound endpoint"));
    let mut leaked = [0u8; 1];
    assert!(
        tokio::time::timeout(Duration::from_millis(20), server.read(&mut leaked))
            .await
            .is_err()
    );
}

struct GuardedWriter<W> {
    inner: W,
    verifier_ran: Arc<AtomicBool>,
}

impl<W: AsyncWrite + Unpin> AsyncWrite for GuardedWriter<W> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        assert!(
            self.verifier_ran.load(Ordering::SeqCst),
            "Hello bytes were written before server-auth verifier ran"
        );
        std::pin::Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[tokio::test]
async fn section17_15_ipc_compatibility() {
    let hello = proto::client_hello("store");
    let ack = proto::evaluate_hello(&hello);
    assert!(ack.accepted);
    assert_eq!(
        ack.required_capabilities,
        proto::daemon_required_capabilities()
    );

    let mut unknown_required = hello.clone();
    unknown_required
        .required_capabilities
        .push("future_required_cap".to_string());
    let rejected = proto::evaluate_hello(&unknown_required);
    assert!(!rejected.accepted);
    assert!(rejected
        .reason
        .as_deref()
        .unwrap_or_default()
        .contains("unknown required capability"));

    let mut missing_required = hello.clone();
    missing_required
        .capabilities
        .retain(|cap| cap != proto::CAP_ADMIN_CAP);
    let rejected = proto::evaluate_hello(&missing_required);
    assert!(!rejected.accepted);
    assert!(rejected
        .reason
        .as_deref()
        .unwrap_or_default()
        .contains("client missing required capability"));

    let mut major_mismatch = hello;
    major_mismatch.protocol_version.major += 1;
    let rejected = proto::evaluate_hello(&major_mismatch);
    assert!(!rejected.accepted);
    assert!(rejected
        .reason
        .as_deref()
        .unwrap_or_default()
        .contains("protocol major mismatch"));

    assert!(serde_json::from_value::<Request>(serde_json::json!({
        "op": "future_required"
    }))
    .is_err());
    let row = proto::COMPATIBILITY_TABLE
        .iter()
        .find(|row| row.protocol_major == proto::PROTOCOL_MAJOR)
        .expect("current compatibility row");
    assert_eq!(row.unknown_operation_error, proto::ERROR_INCOMPATIBLE);

    let ack_frame = Response::Ack {
        message: Some("ack".to_string()),
        delivery_outcome: Some(DeliveryOutcome::AckNoOp),
        address: Some("addr:a".to_string()),
        message_id: Some(7),
        lease_epoch: Some(3),
    };
    let ack_json = serde_json::to_string(&ack_frame).expect("ack json");
    assert!(ack_json.contains("ack-no-op"));
    assert!(ack_json.contains("addr:a"));
    assert!(ack_json.contains("\"message_id\":7"));

    let needs_attach = proto::needs_attach("session must re-attach");
    let needs_json = serde_json::to_string(&needs_attach).expect("needs json");
    assert!(needs_json.contains(proto::ERROR_NEEDS_ATTACH));
}

#[test]
fn section17_16_protocol_major_parallel() {
    let root = std::env::current_dir()
        .expect("current dir")
        .join("target")
        .join("daemon-core-sqlite-tests")
        .join(format!("section17-16-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create root");
    let run_dir = root.join("run");
    let current = DaemonPaths::for_key(
        SingletonKey::from_parts("test-user", root.join("config"), proto::PROTOCOL_MAJOR),
        run_dir.clone(),
    );
    let next = DaemonPaths::for_key(
        SingletonKey::from_parts("test-user", root.join("config"), proto::PROTOCOL_MAJOR + 1),
        run_dir,
    );

    assert_ne!(current.singleton_hash, next.singleton_hash);
    assert_ne!(current.endpoint.display(), next.endpoint.display());
    assert_ne!(current.cap_path, next.cap_path);
    assert!(current
        .cap_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .contains(&current.singleton_hash));
    assert!(next
        .cap_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .contains(&next.singleton_hash));
}

#[tokio::test]
async fn section17_17_durable_backend_clock() {
    let daemon = TestDaemon::new("section17-17");
    let db = daemon.store_path("clock");
    {
        let backend = SqliteBackend::open(&db.to_string_lossy()).expect("open clock db");
        backend.init_schema().await.expect("init clock db");
        let first = backend.durable_clock_now_ms().await.expect("clock first");
        let second = backend.durable_clock_now_ms().await.expect("clock second");
        assert!(second >= first);
    }

    let future_hwm = now_ms() + 60_000;
    {
        let conn = rusqlite::Connection::open(&db).expect("open raw clock db");
        conn.execute(
            "UPDATE clock_hwm SET hwm_ms=?1 WHERE id=1",
            rusqlite::params![future_hwm],
        )
        .expect("set future hwm");
    }
    {
        let reopened = SqliteBackend::open(&db.to_string_lossy()).expect("reopen clock db");
        reopened
            .init_schema()
            .await
            .expect("init reopened clock db");
        let after_reopen = reopened
            .durable_clock_now_ms()
            .await
            .expect("clock after reopen");
        assert!(
            after_reopen > future_hwm,
            "SQLite BackendClock high-water must not move backward across reopen"
        );
    }
}

#[tokio::test]
async fn section17_17_reset_after_restart_clears_wedged_durable_owner() {
    let store;
    let epoch;
    {
        let daemon = TestDaemon::new("section17-17-reset-one");
        store = daemon.store_key("wedged-reset");
        (epoch, _) = registered_epoch(&daemon, &store, "s1", "addr:a").await;
    }

    let restarted = TestDaemon::new("section17-17-reset-two");
    match restarted.reset(&store, "addr:a").await {
        Response::Ack { lease_epoch, .. } => assert_eq!(lease_epoch, Some(epoch)),
        other => panic!("expected reset ack, got {other:?}"),
    }
    let backend = restarted.backend(&store).await.expect("backend");
    let lease = backend
        .get_lease("addr:a")
        .await
        .expect("lease")
        .expect("lease row");
    assert_eq!(lease.lease_epoch, Some(epoch));
    assert_eq!(lease.owner_instance_id, None);
    let (next_epoch, _) = registered_epoch(&restarted, &store, "s2", "addr:a").await;
    assert_eq!(next_epoch, epoch + 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn section17_18_cross_store_from_ambiguity() {
    let daemon = TestDaemon::new("section17-18");
    let store_a = daemon.store_key("store-a");
    let store_b = daemon.store_key("store-b");
    registered_epoch(&daemon, &store_a, "same-session", "addr:a").await;
    registered_epoch(&daemon, &store_b, "same-session", "addr:b").await;

    let waiter_a_daemon = daemon.clone();
    let waiter_a_store = store_a.clone();
    let waiter_a = tokio::spawn(async move {
        waiter_a_daemon
            .wait(&waiter_a_store, "same-session", "addr:a", 5_000)
            .await
    });
    let waiter_b_daemon = daemon.clone();
    let waiter_b_store = store_b.clone();
    let waiter_b = tokio::spawn(async move {
        waiter_b_daemon
            .wait(&waiter_b_store, "same-session", "addr:b", 5_000)
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(matches!(
        daemon.session_end(&store_a, "same-session").await,
        Response::Ack { .. }
    ));
    assert!(matches!(
        waiter_a.await.expect("waiter a"),
        Response::PresenceEnded
    ));
    let backend_b = daemon.backend(&store_b).await.expect("backend b");
    let message_id = insert_message(&backend_b, "addr:b", None).await;
    assert!(matches!(
        waiter_b.await.expect("waiter b"),
        Response::Message { id, .. } if id == message_id
    ));

    registered_epoch(&daemon, &store_a, "multi", "addr:x").await;
    registered_epoch(&daemon, &store_a, "multi", "addr:y").await;
    assert_error_code(
        daemon
            .request(test_support::send_request(
                &store_a,
                "multi",
                None,
                "dest",
                None,
                "ambiguous",
            ))
            .await,
        proto::ERROR_AMBIGUOUS,
    );
    assert_needs_attach(
        daemon
            .request(test_support::send_request(
                &store_a, "unknown", None, "dest", None, "unknown",
            ))
            .await,
    );
    assert!(matches!(
        daemon.session_end(&store_a, "reuse").await,
        Response::Ack { .. }
    ));
    registered_epoch(&daemon, &store_a, "reuse", "addr:reuse-a").await;
    assert!(matches!(
        daemon.session_end(&store_a, "reuse").await,
        Response::Ack { .. }
    ));
    registered_epoch(&daemon, &store_a, "reuse", "addr:reuse-b").await;
    let status = daemon.status().await;
    assert!(status
        .recent_errors
        .iter()
        .any(|e| e.kind == "SessionIdReuse" && e.message.contains("SESSION_ID_REUSE_TRIPWIRE")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn section17_19_delivery_budget() {
    let budget = load_delivery_budget();
    assert!(budget.fence_latency_ms.p95 > 0);
    assert!(budget.fence_latency_ms.p99 >= budget.fence_latency_ms.p95);
    assert!(budget.retention.max_delivery_rows > 0);
    assert!(budget.retention.max_in_flight_entries > 0);
    assert!(budget.workload.single_deliveries > 0);

    let daemon = TestDaemon::new("section17-19");
    let store = daemon.store_key("budget");
    registered_epoch(&daemon, &store, "sender", "addr:sender").await;
    for i in 0..budget.workload.single_deliveries {
        registered_epoch(&daemon, &store, &format!("s{i}"), &format!("addr:{i}")).await;
    }

    let mut emitted = Vec::new();
    for i in 0..budget.workload.single_deliveries {
        let address = format!("addr:{i}");
        let start = Instant::now();
        let sent = daemon
            .request(test_support::send_request(
                &store,
                "sender",
                Some("addr:sender"),
                &address,
                None,
                "budget",
            ))
            .await;
        let message_id = match sent {
            Response::Sent { receipt } => receipt.id,
            other => panic!("expected Sent, got {other:?}"),
        };
        emitted.push((i, address, message_id, start));
    }

    let (_, pending_after_emit) = delivery_counts(&store);
    assert!(
        pending_after_emit <= budget.retention.max_in_flight_entries as i64,
        "pending deliveries {pending_after_emit} exceeded max_in_flight_entries {}",
        budget.retention.max_in_flight_entries
    );

    let mut tasks = Vec::new();
    for (i, address, message_id, start) in emitted {
        let worker = daemon.clone();
        let worker_store = store.clone();
        tasks.push(tokio::spawn(async move {
            let wait = worker
                .wait(&worker_store, &format!("s{i}"), &address, 5_000)
                .await;
            assert!(matches!(wait, Response::Message { id, .. } if id == message_id));
            assert_ack_outcome(
                worker
                    .ack(&worker_store, &format!("s{i}"), &address, message_id)
                    .await,
                DeliveryOutcome::Marked,
            );
            start.elapsed()
        }));
    }

    let mut latencies_ms = Vec::new();
    for task in tasks {
        latencies_ms.push(task.await.expect("budget worker").as_millis() as u64);
    }
    latencies_ms.sort_unstable();
    let p95 = percentile_nearest_rank(&latencies_ms, 95);
    let p99 = percentile_nearest_rank(&latencies_ms, 99);
    assert!(
        p95 <= budget.fence_latency_ms.p95,
        "p95 fence latency {p95} ms exceeded {} ms; samples={latencies_ms:?}",
        budget.fence_latency_ms.p95
    );
    assert!(
        p99 <= budget.fence_latency_ms.p99,
        "p99 fence latency {p99} ms exceeded {} ms; samples={latencies_ms:?}",
        budget.fence_latency_ms.p99
    );
    let (retained, pending) = delivery_counts(&store);
    assert_eq!(pending, 0, "all budget deliveries should be ack-marked");
    assert!(
        retained <= budget.retention.max_delivery_rows as i64,
        "retained delivery rows {retained} exceeded {}",
        budget.retention.max_delivery_rows
    );
}

#[derive(Debug, Deserialize)]
struct DeliveryBudget {
    fence_latency_ms: FenceLatencyBudget,
    retention: RetentionBudget,
    workload: WorkloadBudget,
}

#[derive(Debug, Deserialize)]
struct FenceLatencyBudget {
    p95: u64,
    p99: u64,
}

#[derive(Debug, Deserialize)]
struct RetentionBudget {
    max_delivery_rows: u64,
    max_in_flight_entries: u64,
}

#[derive(Debug, Deserialize)]
struct WorkloadBudget {
    single_deliveries: u64,
}

fn load_delivery_budget() -> DeliveryBudget {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("docs")
        .join("design")
        .join("daemon-delivery-budget.toml");
    let text = std::fs::read_to_string(&path).expect("read delivery budget artifact");
    toml::from_str(&text).expect("parse delivery budget artifact")
}

fn percentile_nearest_rank(sorted: &[u64], percentile: u64) -> u64 {
    assert!(!sorted.is_empty());
    let len = sorted.len() as u64;
    let rank = ((percentile * len).saturating_add(99) / 100).max(1);
    sorted[(rank as usize).saturating_sub(1).min(sorted.len() - 1)]
}
