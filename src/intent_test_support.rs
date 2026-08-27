//! Test harness for the station-intent path (issue #106 / ADR 0052).
//!
//! Two things the existing process/core suites could not do are provided here:
//!
//! 1. **A fake producer endpoint.** The daemon restores a push registration only after proving a
//!    producer is alive over the real transport (named pipe on Windows, unix socket elsewhere).
//!    Without a controllable producer, every reconcile test would either need a live Copilot
//!    session or would have to stub out the exact code path most worth testing. The fake speaks the
//!    real probe protocol and has knobs for each failure the reconciler must distinguish.
//! 2. **Intent fixtures.** Building a `Live` intent by hand in each test would duplicate the
//!    identity-capture rules and let a test drift from what attach actually writes.
//!
//! `#[doc(hidden)]`: this is a test seam, not public API.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::daemon_ipc::IntentRecoveryState;
use crate::platform_fs;
use crate::station_intent::{
    CredentialDescriptorV1, DaemonCompat, HandlerDescriptorV1, IntentEvidence,
    ProducerDescriptorV1, ProducerTransport, ProtocolRange, StationIntentV1,
    CREDENTIAL_KIND_OWNER_PRIVATE_JSON_FIELD_V1, PRODUCER_KIND_LOCAL_ENDPOINT_CHALLENGE_V1,
    STATION_INTENT_SCHEMA_VERSION,
};

/// How the fake producer should answer a probe. Each variant maps to exactly one outcome the
/// reconciler must distinguish, so a test names the *cause* rather than a magic payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProducerBehavior {
    /// Answers correctly: echoes the nonce, names the session, advertises protocol 2.
    Healthy,
    /// Echoes a different nonce — a replayed or forged answer.
    WrongNonce,
    /// Answers for a different session.
    WrongSession,
    /// Advertises protocol 1: a *legacy* producer, not a failed one.
    LegacyProtocol,
    /// Rejects every secret, as a producer whose secret rotated would.
    RejectSecret,
    /// Accepts the connection and never answers.
    Hang,
}

/// A controllable producer endpoint that speaks the real probe protocol over the real transport.
pub struct FakeProducer {
    endpoint_path: String,
    secret: String,
    session_id: String,
    shutdown: Arc<AtomicBool>,
    #[cfg(unix)]
    socket_path: PathBuf,
}

impl FakeProducer {
    /// Start a producer for `session_id` at a transport-appropriate endpoint under `dir`.
    pub async fn start(
        dir: &Path,
        session_id: &str,
        secret: &str,
        behavior: ProducerBehavior,
    ) -> Self {
        use std::sync::atomic::AtomicU64;
        static NEXT: AtomicU64 = AtomicU64::new(1);
        // Endpoint names must be unique per producer instance: tests reuse session ids, and a
        // Windows named pipe name collides process-wide (a second `first_pipe_instance` bind of the
        // same name fails with access denied).
        let unique = NEXT.fetch_add(1, Ordering::SeqCst);
        let shutdown = Arc::new(AtomicBool::new(false));
        #[cfg(windows)]
        let endpoint_path = format!(
            r"\\.\pipe\telex-fake-producer-{}-{unique}",
            std::process::id()
        );
        #[cfg(unix)]
        let socket_path =
            PathBuf::from("/tmp").join(format!("telex-fp-{}-{unique}.sock", std::process::id()));
        #[cfg(unix)]
        let endpoint_path = socket_path.to_string_lossy().into_owned();
        let _ = dir;

        let producer = Self {
            endpoint_path: endpoint_path.clone(),
            secret: secret.to_string(),
            session_id: session_id.to_string(),
            shutdown: shutdown.clone(),
            #[cfg(unix)]
            socket_path: socket_path.clone(),
        };
        producer.spawn_server(behavior).await;
        producer
    }

    pub fn endpoint_path(&self) -> &str {
        &self.endpoint_path
    }

    pub fn secret(&self) -> &str {
        &self.secret
    }

    /// Stop answering. Models a producer that died without cleaning up.
    pub fn kill(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        #[cfg(unix)]
        let _ = std::fs::remove_file(&self.socket_path);
    }

    fn answer(&self, behavior: ProducerBehavior, request: &serde_json::Value) -> serde_json::Value {
        let provided = request.get("secret").and_then(|v| v.as_str()).unwrap_or("");
        if behavior == ProducerBehavior::RejectSecret || provided != self.secret {
            return serde_json::json!({"ok": false, "error": "unauthorized"});
        }
        if request.get("op").and_then(|v| v.as_str()) != Some("probe") {
            return serde_json::json!({"ok": false, "error": "unsupported_op"});
        }
        let nonce = request.get("nonce").and_then(|v| v.as_str()).unwrap_or("");
        match behavior {
            ProducerBehavior::LegacyProtocol => {
                serde_json::json!({"ok": false, "error": "unsupported_op"})
            }
            ProducerBehavior::WrongNonce => serde_json::json!({
                "ok": true,
                "nonce": "0000000000000000",
                "sessionId": self.session_id,
                "protocol": 2,
                "bridgeGeneration": 1,
            }),
            ProducerBehavior::WrongSession => serde_json::json!({
                "ok": true,
                "nonce": nonce,
                "sessionId": format!("{}-imposter", self.session_id),
                "protocol": 2,
                "bridgeGeneration": 1,
            }),
            _ => serde_json::json!({
                "ok": true,
                "nonce": nonce,
                "sessionId": self.session_id,
                "protocol": 2,
                "bridgeGeneration": 1,
            }),
        }
    }

    #[cfg(unix)]
    async fn spawn_server(&self, behavior: ProducerBehavior) {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixListener;

        let _ = std::fs::remove_file(&self.socket_path);
        let listener = UnixListener::bind(&self.socket_path).expect("bind fake producer socket");
        let shutdown = self.shutdown.clone();
        let secret = self.secret.clone();
        let session_id = self.session_id.clone();
        let socket_path = self.socket_path.clone();
        tokio::spawn(async move {
            let responder = FakeProducer {
                endpoint_path: socket_path.to_string_lossy().into_owned(),
                secret,
                session_id,
                shutdown: shutdown.clone(),
                socket_path,
            };
            loop {
                if shutdown.load(Ordering::SeqCst) {
                    return;
                }
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                // See the Windows note: a killed producer must not answer a connection that was
                // already pending.
                if shutdown.load(Ordering::SeqCst) {
                    return;
                }
                if behavior == ProducerBehavior::Hang {
                    // Hold the connection open without answering; the reconciler's probe timeout
                    // is what must bound this.
                    std::mem::forget(stream);
                    continue;
                }
                let (read_half, mut write_half) = tokio::io::split(stream);
                let mut reader = BufReader::new(read_half);
                let mut line = String::new();
                if reader.read_line(&mut line).await.is_err() {
                    continue;
                }
                let request: serde_json::Value =
                    serde_json::from_str(line.trim()).unwrap_or(serde_json::Value::Null);
                let response = responder.answer(behavior, &request);
                let _ = write_half
                    .write_all(format!("{response}\n").as_bytes())
                    .await;
                let _ = write_half.flush().await;
            }
        });
    }

    #[cfg(windows)]
    async fn spawn_server(&self, behavior: ProducerBehavior) {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::windows::named_pipe::ServerOptions;

        let pipe_name = self.endpoint_path.clone();
        let shutdown = self.shutdown.clone();
        let secret = self.secret.clone();
        let session_id = self.session_id.clone();
        let first = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&pipe_name)
            .expect("bind fake producer pipe");
        tokio::spawn(async move {
            let responder = FakeProducer {
                endpoint_path: pipe_name.clone(),
                secret,
                session_id,
                shutdown: shutdown.clone(),
            };
            let mut server = first;
            loop {
                if shutdown.load(Ordering::SeqCst) {
                    return;
                }
                if server.connect().await.is_err() {
                    return;
                }
                // A killed producer stops answering *immediately*, including on a connection that
                // was already pending: the pipe instance stays bound until the process exits, so
                // checking only at the top of the loop would let a "dead" producer answer one more
                // probe and make a liveness test pass for the wrong reason.
                if shutdown.load(Ordering::SeqCst) {
                    return;
                }
                let next = match ServerOptions::new().create(&pipe_name) {
                    Ok(next) => next,
                    Err(_) => return,
                };
                let connected = std::mem::replace(&mut server, next);
                if behavior == ProducerBehavior::Hang {
                    std::mem::forget(connected);
                    continue;
                }
                let (read_half, mut write_half) = tokio::io::split(connected);
                let mut reader = BufReader::new(read_half);
                let mut line = String::new();
                if reader.read_line(&mut line).await.is_err() {
                    continue;
                }
                let request: serde_json::Value =
                    serde_json::from_str(line.trim()).unwrap_or(serde_json::Value::Null);
                let response = responder.answer(behavior, &request);
                let _ = write_half
                    .write_all(format!("{response}\n").as_bytes())
                    .await;
                let _ = write_half.flush().await;
            }
        });
    }
}

impl Drop for FakeProducer {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Write an owner-private credential file holding `secret` at `/secret`.
pub fn write_credential_file(path: &Path, secret: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("credential parent");
    }
    let body = serde_json::json!({"sessionId": "fixture", "secret": secret});
    let bytes = serde_json::to_vec(&body).expect("encode credential");
    let _ = std::fs::remove_file(path);
    platform_fs::write_owner_only_file_atomic(path, &bytes).expect("write credential");
}

/// Register a producer root for tests and return its canonical path.
pub fn register_test_producer_root(root_id: &str, path: &Path) -> PathBuf {
    let canonical =
        platform_fs::ensure_owner_private_producer_root(path).expect("secure test producer root");
    crate::handler_kinds::register_producer_root(root_id, canonical.clone());
    canonical
}

/// Register the handler kind tests use. Idempotent.
pub fn register_test_handler_kind() {
    crate::handler_kinds::register_handler_kind(crate::handler_kinds::HandlerKind {
        id: crate::handler_kinds::COPILOT_PUSH_HANDLER_KIND,
    });
}

/// Build a `Live` intent that points at a running [`FakeProducer`] and is verifiable *by this
/// process*: the producer identity is this process's own pid/start-time/executable, and the host
/// and boot identities are the real local ones. Anything a test wants to make unverifiable it
/// mutates explicitly, so the reason a test fails is always visible in the test.
#[allow(clippy::too_many_arguments)]
pub fn live_intent(
    store_key: &str,
    session_id: &str,
    address: &str,
    singleton_hash: &str,
    producer: &FakeProducer,
    credential_root_id: &str,
    credential_path: &Path,
) -> StationIntentV1 {
    let pid = std::process::id();
    let now = crate::model::now_ms();
    StationIntentV1 {
        schema_version: STATION_INTENT_SCHEMA_VERSION,
        generation: 1,
        created_at_ms: now,
        updated_at_ms: now,
        state: IntentRecoveryState::Live,
        store_key: store_key.to_string(),
        session_id: session_id.to_string(),
        address: address.to_string(),
        occupant: "fixture-occupant".to_string(),
        description: Some("fixture station".to_string()),
        scope: None,
        tags: None,
        delivery_mode: "push".to_string(),
        wake_on_cc: false,
        cc_watermark_ms: None,
        handler: HandlerDescriptorV1 {
            kind: crate::handler_kinds::COPILOT_PUSH_HANDLER_KIND.to_string(),
            session_id: session_id.to_string(),
        },
        producer: ProducerDescriptorV1 {
            kind: PRODUCER_KIND_LOCAL_ENDPOINT_CHALLENGE_V1.to_string(),
            transport: if cfg!(windows) {
                ProducerTransport::NamedPipe
            } else {
                ProducerTransport::UnixSocket
            },
            endpoint_path: producer.endpoint_path().to_string(),
            exe_path: platform_fs::process_exe_path(pid).expect("own exe path"),
            pid,
            start_time: crate::session_watch::capture_process_start_time(pid)
                .expect("own start time"),
            host_id: platform_fs::host_id().expect("host id"),
            boot_id: platform_fs::boot_id().expect("boot id"),
            protocol: ProtocolRange { min: 2, max: 2 },
            credential: CredentialDescriptorV1 {
                kind: CREDENTIAL_KIND_OWNER_PRIVATE_JSON_FIELD_V1.to_string(),
                root_id: credential_root_id.to_string(),
                path: credential_path.to_path_buf(),
                pointer: "/secret".to_string(),
                max_age_ms: 60_000,
            },
        },
        daemon_compat: DaemonCompat {
            protocol_major: crate::daemon_ipc::PROTOCOL_MAJOR,
            protocol_minor: crate::daemon_ipc::PROTOCOL_MINOR,
        },
        singleton_hash: singleton_hash.to_string(),
        evidence: IntentEvidence::default(),
        armed: None,
        extra: Default::default(),
    }
}

/// Build the `Pending` intent a *first* attach writes, with no producer at all.
///
/// This is deliberately producer-free: on a first attach the bridge extension has been written but
/// not loaded, so the record carries the placeholder identity and a credential path that does not
/// exist yet. Tests about the daemon's arming-proof transaction need exactly that shape and must
/// not have to stand up a [`FakeProducer`] to get it — the record is never reconciled in this state,
/// so no probe is ever attempted against it.
pub fn pending_intent(
    store_key: &str,
    session_id: &str,
    address: &str,
    singleton_hash: &str,
) -> StationIntentV1 {
    let now = crate::model::now_ms();
    StationIntentV1 {
        schema_version: STATION_INTENT_SCHEMA_VERSION,
        generation: 1,
        created_at_ms: now,
        updated_at_ms: now,
        state: IntentRecoveryState::Pending,
        store_key: store_key.to_string(),
        session_id: session_id.to_string(),
        address: address.to_string(),
        occupant: "fixture-occupant".to_string(),
        description: None,
        scope: None,
        tags: None,
        delivery_mode: "push".to_string(),
        wake_on_cc: false,
        cc_watermark_ms: None,
        handler: HandlerDescriptorV1 {
            kind: crate::handler_kinds::COPILOT_PUSH_HANDLER_KIND.to_string(),
            session_id: session_id.to_string(),
        },
        producer: ProducerDescriptorV1 {
            kind: PRODUCER_KIND_LOCAL_ENDPOINT_CHALLENGE_V1.to_string(),
            transport: if cfg!(windows) {
                ProducerTransport::NamedPipe
            } else {
                ProducerTransport::UnixSocket
            },
            endpoint_path: format!("telex-fixture-{session_id}"),
            exe_path: PathBuf::from("not-loaded-yet"),
            pid: 0,
            start_time: 0,
            host_id: String::new(),
            boot_id: String::new(),
            protocol: ProtocolRange { min: 2, max: 2 },
            credential: CredentialDescriptorV1 {
                kind: CREDENTIAL_KIND_OWNER_PRIVATE_JSON_FIELD_V1.to_string(),
                root_id: "copilot_bridge_root".to_string(),
                path: PathBuf::from("not-created-yet.json"),
                pointer: "/secret".to_string(),
                max_age_ms: 60_000,
            },
        },
        daemon_compat: DaemonCompat {
            protocol_major: crate::daemon_ipc::PROTOCOL_MAJOR,
            protocol_minor: crate::daemon_ipc::PROTOCOL_MINOR,
        },
        singleton_hash: singleton_hash.to_string(),
        evidence: IntentEvidence::default(),
        armed: None,
        extra: Default::default(),
    }
}
