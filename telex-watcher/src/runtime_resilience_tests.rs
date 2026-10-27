//! Real daemon and backend faults; assertions use the running owner and its store.

use crate::evidence::*;
use crate::isolation::{self, Isolation};
use crate::management::{Command, LocalManagement, Response};
use crate::{Runtime, RuntimeConfig};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use telex::backend::Backend;
use telex::profiles::ConfigFile;
use tokio::time::{sleep, timeout, Instant};

const PROFILE: &str = "resilience-test";
const WATCH: &str = "resilience";
const TARGET: &str = "agent:resilience-recipient";
const DEADLINE: Duration = Duration::from_secs(100);

struct OwnerTask(tokio::task::JoinHandle<anyhow::Result<()>>);

impl Drop for OwnerTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct WithheldSelector {
    original: std::path::PathBuf,
    withheld: std::path::PathBuf,
}

impl WithheldSelector {
    fn restore(&self) {
        if self.withheld.exists() {
            std::fs::rename(&self.withheld, &self.original).unwrap();
        }
    }
}

impl Drop for WithheldSelector {
    fn drop(&mut self) {
        self.restore();
    }
}

async fn health(management: &LocalManagement) -> RuntimeHealth {
    match timeout(Duration::from_secs(45), management.execute(Command::Health))
        .await
        .expect("health request deadline")
        .expect("runtime must keep serving management")
    {
        Response::Health(value) => value,
        other => panic!("expected runtime health, got {other:?}"),
    }
}

async fn watch(management: &LocalManagement) -> Watch {
    match management
        .execute(Command::Show { id: WATCH.into() })
        .await
        .unwrap()
    {
        Response::Watch(Some(value)) => value,
        other => panic!("expected watch, got {other:?}"),
    }
}

async fn attempts(management: &LocalManagement) -> Vec<Attempt> {
    match management
        .execute(Command::Attempts {
            id: WATCH.into(),
            limit: 1000,
        })
        .await
        .unwrap()
    {
        Response::Attempts(value) => value,
        other => panic!("expected attempts, got {other:?}"),
    }
}

async fn pending(management: &LocalManagement) -> Option<Box<Operation>> {
    match management
        .execute(Command::Pending { id: WATCH.into() })
        .await
        .unwrap()
    {
        Response::Pending(value) => value,
        other => panic!("expected pending operation, got {other:?}"),
    }
}

async fn wait_for_state(management: &LocalManagement, cursor: u64) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let value = watch(management).await;
        if value.state == json!(cursor) {
            assert_eq!(value.last_committed_event_sequence, cursor);
            assert!(value.pending_operation_id.is_none());
            return;
        }
        if Instant::now() >= deadline {
            panic!(
                "state did not advance: {value:?}; runtime: {:?}",
                health(management).await
            );
        }
        sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_detector(iso: &Isolation, management: &LocalManagement) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let value = watch(management).await;
        if value
            .running_attempt_id
            .as_ref()
            .is_some_and(|id| iso.root.join(format!("started-{id}")).is_file())
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "detector did not start: {value:?}"
        );
        sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_sender(management: &LocalManagement, ready: bool) -> RuntimeHealth {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let value = health(management).await;
        if value
            .senders
            .iter()
            .any(|sender| sender.backend_profile == PROFILE && sender.ready == ready)
        {
            assert_eq!(value.production_ready, ready, "{value:?}");
            if !ready {
                assert!(value.senders[0].error.is_some(), "{value:?}");
                assert!(value
                    .diagnostics
                    .contains(&RuntimeDiagnostic::SenderUnready));
            }
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "sender readiness stayed wrong: {value:?}"
        );
        sleep(Duration::from_millis(100)).await;
    }
}

async fn assert_deliveries(management: &LocalManagement, store: &dyn Backend, count: u64) {
    let events = match management
        .execute(Command::Events {
            id: WATCH.into(),
            limit: 1000,
        })
        .await
        .unwrap()
    {
        Response::Events(value) => value,
        other => panic!("expected events, got {other:?}"),
    };
    let accepted: Vec<_> = events
        .into_iter()
        .filter(|event| event.status == OperationStatus::Accepted)
        .collect();
    assert_eq!(accepted.len() as u64, count);
    let messages = store.export(Some(TARGET), None, 0).await.unwrap();
    assert_eq!(
        messages.len() as u64,
        count,
        "duplicate or missing durable delivery"
    );
    let mut sequences = BTreeSet::new();
    let mut operations = BTreeSet::new();
    let mut receipts = BTreeSet::new();
    for event in accepted {
        let receipt = event.receipt.as_ref().expect("accepted receipt");
        assert!(sequences.insert(event.event_sequence));
        assert!(operations.insert(event.operation_id.clone()));
        assert!(receipts.insert(receipt.message_id));
        assert!(
            messages.iter().any(|message| {
                let metadata: EventMetadata =
                    serde_json::from_str(message.metadata.as_deref().unwrap()).unwrap();
                message.id == receipt.message_id
                    && metadata.watcher.operation_id == event.operation_id
                    && metadata.watcher.event_id == event.event_id
                    && metadata.watcher.event_sequence == event.event_sequence
            }),
            "receipt has no matching durable message"
        );
    }
    assert_eq!(sequences, (1..=count).collect());
}

async fn assert_outage(
    management: &LocalManagement,
    owner: &tokio::task::JoinHandle<anyhow::Result<()>>,
    runtime_id: &str,
    store: &dyn Backend,
) {
    let before = wait_for_sender(management, false).await;
    assert_eq!(before.runtime_id, runtime_id);
    let deadline = Instant::now() + DEADLINE;
    while watch(management).await.running_attempt_id.is_some() {
        assert!(
            Instant::now() < deadline,
            "in-flight detector did not finish"
        );
        sleep(Duration::from_millis(50)).await;
    }
    let attempt_count = attempts(management).await.len();
    assert!(
        attempts(management).await.iter().any(|attempt| {
            matches!(
                attempt.outcome,
                AttemptOutcome::Failed | AttemptOutcome::Pending
            ) && !attempt.state_committed
        }),
        "the in-flight event must not commit its proposed state without a receipt"
    );
    assert_eq!(watch(management).await.state, json!(1));
    assert_deliveries(management, store, 1).await;
    let budget = pending(management)
        .await
        .map(|operation| operation.send_budget);
    if let Some(budget) = &budget {
        assert!(
            budget.used <= 1,
            "sender failure did not fence subsequent sends: {budget:?}"
        );
    }
    // Three send attempts would exhaust after 1 + 2 + 4 seconds; wait beyond that.
    sleep(Duration::from_secs(10)).await;
    let after = health(management).await;
    assert!(!owner.is_finished(), "runtime ended during sender outage");
    assert_eq!(after.runtime_id, runtime_id);
    assert_eq!(after.status, RuntimeStatus::Running);
    assert!(after.heartbeat_at > before.heartbeat_at, "{after:?}");
    assert!(!after.senders[0].ready);
    assert_eq!(attempts(management).await.len(), attempt_count);
    let value = watch(management).await;
    assert_eq!(value.state, json!(1));
    assert_eq!(value.lifecycle, Lifecycle::Active, "{value:?}");
    if let Some(before) = budget {
        let after = pending(management)
            .await
            .expect("pending identity retained")
            .send_budget;
        assert_eq!(
            after.remaining, before.remaining,
            "unready sender spent retry budget"
        );
        assert_eq!(after.used, before.used, "unready sender attempted a send");
    }
}

async fn exercise(
    iso: &Isolation,
    store: &dyn Backend,
    backend_fault: Option<&ConnectionFault>,
    release_before_observation: bool,
) {
    #[cfg(unix)]
    {
        let telex::daemon::Endpoint::UnixSocket(path) =
            telex::daemon::DaemonPaths::current().unwrap().endpoint;
        std::os::unix::net::SocketAddr::from_pathname(&path)
            .unwrap_or_else(|error| panic!("isolated daemon endpoint {}: {error}", path.display()));
    }
    let (runtime, management) = Runtime::open(RuntimeConfig::installed(
        iso.root.join("registry.sqlite"),
        iso.trusted_root(),
    ))
    .unwrap();
    let runtime_id = runtime.runtime_id().to_owned();
    let mut owner = OwnerTask(tokio::spawn(runtime.run()));
    let detector = isolation::branch_binary().with_file_name(format!(
        "watcher_resilience_detector{}",
        std::env::consts::EXE_SUFFIX
    ));
    assert!(
        detector.is_absolute() && detector.is_file(),
        "build workspace binaries first"
    );
    std::fs::write(iso.root.join("permit-0"), b"ready").unwrap();
    let registration = json!({
        "schemaVersion": 2, "id": WATCH, "command": [detector, iso.root],
        "intervalSeconds": 1, "timeoutSeconds": 100,
        "backendProfile": PROFILE, "sender": "service:resilience", "target": TARGET,
        "initialState": 0
    });
    let response = management
        .execute(Command::Register {
            json: serde_json::to_vec(&registration).unwrap(),
            working_directory: iso.root.clone(),
        })
        .await
        .unwrap();
    assert!(
        matches!(response, Response::Mutation(ref value) if value.refusal.is_none()),
        "{response:?}"
    );
    wait_for_sender(&management, true).await;
    wait_for_state(&management, 1).await;
    wait_for_detector(iso, &management).await;
    let first = wait_for_sender(&management, true).await;
    let daemon_pid = iso.daemon_pid().expect("isolated daemon is running");
    let binary = iso.current_binary();
    let selector = WithheldSelector {
        original: iso.layout().current_path,
        withheld: iso.root.join("withheld-current"),
    };
    if let Some(fault) = backend_fault {
        fault.set_available(false).await;
    } else {
        // Keep connect-or-spawn unavailable, so the outage cannot heal before inspection.
        std::fs::rename(&selector.original, &selector.withheld).unwrap();
        iso.stop_daemon_with_executable(&binary);
    }
    if !release_before_observation {
        wait_for_sender(&management, false).await;
    }
    std::fs::write(iso.root.join("permit-1"), b"ready").unwrap();
    assert_outage(&management, &owner.0, &runtime_id, store).await;
    let pending = watch(&management).await.pending_operation_id;
    if let Some(fault) = backend_fault {
        fault.set_available(true).await;
    } else {
        selector.restore();
    }
    let recovered = wait_for_sender(&management, true).await;
    assert_eq!(recovered.runtime_id, first.runtime_id);
    assert!(recovered.heartbeat_at > first.heartbeat_at);
    assert!(!owner.0.is_finished());
    if backend_fault.is_none() {
        assert_ne!(
            iso.daemon_pid().unwrap(),
            daemon_pid,
            "daemon was not replaced"
        );
    }
    wait_for_state(&management, 2).await;
    assert_deliveries(&management, store, 2).await;
    if let Some(operation_id) = pending {
        let Response::Events(events) = management
            .execute(Command::Events {
                id: WATCH.into(),
                limit: 1000,
            })
            .await
            .unwrap()
        else {
            panic!("expected event evidence")
        };
        assert!(
            events.iter().any(|event| {
                event.operation_id == operation_id && event.status == OperationStatus::Accepted
            }),
            "recovery must preserve the staged identity"
        );
    }
    wait_for_detector(iso, &management).await;
    std::fs::write(iso.root.join("permit-2"), b"ready").unwrap();
    wait_for_state(&management, 3).await;
    assert_deliveries(&management, store, 3).await;
    management.execute(Command::Shutdown).await.unwrap();
    timeout(Duration::from_secs(30), &mut owner.0)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_watcher_backend_resilience() {
    sqlite_proof(true).await;
}

#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_watcher_sender_outage() {
    sqlite_proof(false).await;
}

#[cfg(feature = "sqlite")]
async fn sqlite_proof(release_before_observation: bool) {
    let _lock = isolation::ENV_LOCK.lock().await;
    let iso = Isolation::new("wr-sqlite");
    let _restore = iso.apply_env();
    let profile = telex::profiles::implicit_sqlite(Some(iso.db_path.to_str().unwrap()));
    iso.write_config(&ConfigFile {
        default: None,
        backends: BTreeMap::from([(PROFILE.into(), profile.clone())]),
    });
    let store = telex::profiles::build(&profile, None).await.unwrap();
    store.init_schema().await.unwrap();
    exercise(&iso, store.as_ref(), None, release_before_observation).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_watcher_backend_resilience() {
    postgres_proof(true).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_watcher_sender_outage() {
    postgres_proof(false).await;
}

#[cfg(feature = "postgres")]
async fn postgres_proof(release_before_observation: bool) {
    use telex::backend::postgres::make_tls;
    let _lock = isolation::ENV_LOCK.lock().await;
    let Some(url) = isolation::postgres_url_or_fail_closed("watcher-resilience") else {
        return;
    };
    let mut config: tokio_postgres::Config = url.parse().unwrap();
    if let Ok(password) = std::env::var("TELEX_PG_PASSWORD") {
        config.password(password);
    }
    let host = match config.get_hosts() {
        [tokio_postgres::config::Host::Tcp(host)] => host.clone(),
        _ => panic!("resilience proof requires one disposable loopback PostgreSQL host"),
    };
    let port = config.get_ports().first().copied().unwrap_or(5432);
    let proxy = ConnectionFault::start(host, port).await;
    let mut password = isolation::EnvRestore::default();
    password.set(
        "TELEX_RESILIENCE_PASSWORD",
        std::str::from_utf8(config.get_password().unwrap_or(b"")).unwrap(),
    );
    let quote = |value: &str| value.replace('\\', "\\\\").replace('\'', "\\'");
    for terminate_backend in [false, true] {
        let iso = Isolation::new("wr-pg");
        let _restore = iso.apply_env();
        let schema = format!("resilience_{}", uuid::Uuid::new_v4().simple());
        let mut profile = telex::profiles::implicit_sqlite(None);
        profile.kind = "postgres".into();
        profile.path = None;
        profile.url = Some(url.clone());
        profile.schema = Some(schema.clone());
        profile.auth = Some("password".into());
        if std::env::var("TELEX_PG_PASSWORD").is_ok() {
            profile.password_env = Some("TELEX_PG_PASSWORD".into());
        }
        let store = telex::profiles::build(&profile, None).await.unwrap();
        store.init_schema().await.unwrap();
        profile.url = Some(format!(
            "host=127.0.0.1 port={} user='{}' dbname='{}' sslmode=disable",
            proxy.port,
            quote(config.get_user().unwrap()),
            quote(config.get_dbname().unwrap_or("postgres")),
        ));
        profile.password_env = Some("TELEX_RESILIENCE_PASSWORD".into());
        iso.write_config(&ConfigFile {
            default: None,
            backends: BTreeMap::from([(PROFILE.into(), profile)]),
        });
        exercise(
            &iso,
            store.as_ref(),
            terminate_backend.then_some(&proxy),
            release_before_observation,
        )
        .await;
        iso.stop_daemon();
        drop(store);
        let (admin, connection) = config.connect(make_tls().unwrap()).await.unwrap();
        let connection = tokio::spawn(connection);
        admin
            .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
            .await
            .unwrap();
        drop(admin);
        connection.await.unwrap().unwrap();
    }
}

// Only this fixture's TCP streams cross this boundary; no server or foreign session is stopped.
struct ConnectionFault {
    #[cfg(feature = "postgres")]
    port: u16,
    #[cfg(feature = "postgres")]
    available: tokio::sync::watch::Sender<bool>,
    #[cfg(feature = "postgres")]
    applied: tokio::sync::watch::Receiver<bool>,
    #[cfg(feature = "postgres")]
    task: tokio::task::JoinHandle<()>,
}

impl ConnectionFault {
    #[cfg(feature = "postgres")]
    async fn start(host: String, port: u16) -> Self {
        use tokio::net::{TcpListener, TcpStream};
        assert!(
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback()),
            "resilience fixture requires a disposable loopback PostgreSQL server"
        );
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port_local = listener.local_addr().unwrap().port();
        let (available, mut changes) = tokio::sync::watch::channel(true);
        let (acknowledge, applied) = tokio::sync::watch::channel(true);
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    change = changes.changed() => {
                        if change.is_err() { break; }
                        let available = *changes.borrow_and_update();
                        if !available {
                            assert!(!connections.is_empty(), "fault must terminate live backend connections");
                            connections.abort_all();
                            while let Some(result) = connections.join_next().await {
                                if let Err(error) = result { assert!(error.is_cancelled(), "{error}"); }
                            }
                        }
                        acknowledge.send_replace(available);
                    }
                    accepted = listener.accept() => {
                        let (mut client, _) = accepted.unwrap();
                        if !*changes.borrow() { continue; }
                        let host = host.clone();
                        connections.spawn(async move {
                            let mut server = TcpStream::connect((host.as_str(), port)).await.unwrap();
                            if let Err(error) = tokio::io::copy_bidirectional(&mut client, &mut server).await {
                                assert!(matches!(error.kind(),
                                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                                        | std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::UnexpectedEof),
                                    "unexpected proxy I/O: {error}");
                            }
                        });
                    }
                    Some(result) = connections.join_next(), if !connections.is_empty() => {
                        if let Err(error) = result { assert!(error.is_cancelled(), "{error}"); }
                    }
                }
            }
        });
        Self {
            port: port_local,
            available,
            applied,
            task,
        }
    }

    async fn set_available(&self, available: bool) {
        #[cfg(feature = "postgres")]
        {
            assert!(!self.task.is_finished(), "backend fault boundary stopped");
            self.available.send(available).unwrap();
            let mut applied = self.applied.clone();
            timeout(
                Duration::from_secs(5),
                applied.wait_for(|value| *value == available),
            )
            .await
            .unwrap()
            .unwrap();
        }
        #[cfg(not(feature = "postgres"))]
        panic!("no PostgreSQL fault boundary: {available}");
    }
}

#[cfg(feature = "postgres")]
impl Drop for ConnectionFault {
    fn drop(&mut self) {
        self.task.abort();
    }
}
