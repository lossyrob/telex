use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::Path;
use std::time::Duration;

use crate::cli::{Ctx, WaitArgs};
use crate::daemon_ipc::{NeedsAttachReason, Request, Response, ERROR_NEEDS_ATTACH};
use crate::identity::{default_occupant, resolve_session_id};
use crate::model::now_ms;

const DEFAULT_RECONNECT_GRACE_MS: u64 = 3_000;
const RECONNECT_RETRY_SLEEP_MS: u64 = 50;

pub async fn run(ctx: &Ctx, args: WaitArgs) -> Result<i32> {
    let address = ctx.cfg.require_address(&ctx.address)?;
    let store_key = ctx.store_key()?;
    let session_id = resolve_session_id(args.session.as_deref())?;
    if args.since != 0 {
        eprintln!(
            "[wait] warning: --since is deprecated for daemon-core waits and is currently ignored"
        );
    }
    if args.stale_heartbeat_ms != 15_000 {
        eprintln!(
            "[wait] warning: --stale-heartbeat-ms is deprecated for daemon-core waits and is currently ignored"
        );
    }
    if args.hang_ms != 8_000 {
        eprintln!(
            "[wait] warning: --hang-ms is a finite-timeout watchdog in daemon-core waits and is ignored for unbounded idle waits"
        );
    }

    let cfg = WaitLoopConfig {
        store_key,
        session_id,
        address: address.clone(),
        timeout_ms: args.timeout_ms,
        min_attention: args.min_attention.map(|a| a.as_str().to_string()),
        hang_ms: args.hang_ms,
        reconnect_grace_ms: reconnect_grace_ms(args.reconnect_grace_ms),
        waiter_pid: std::process::id(),
        waiter_start_time: crate::session_watch::capture_process_start_time(std::process::id()),
    };
    if let Some(dir) = args.out_dir.as_deref() {
        if let Err(e) = write_wait_start_artifacts(dir, cfg.waiter_pid) {
            eprintln!(
                "[wait] warning: could not write --out-dir startup artifacts to {}: {e}",
                dir.display()
            );
        }
    }
    let mut connector = RealWaitConnector;
    let outcome = match wait_loop(&mut connector, &cfg).await {
        Ok(WaitTerminal::DaemonGone(message)) => WaitOutcome::daemon_gone(message),
        Ok(WaitTerminal::DaemonHung(message)) => WaitOutcome::daemon_hung(message),
        Ok(WaitTerminal::Response(response)) => WaitOutcome::from_response(response)?,
        Err(e) => {
            if let Some(dir) = args.out_dir.as_deref() {
                if let Err(write_err) =
                    write_wait_artifacts(dir, &WaitOutcome::error(e.to_string()), &address)
                {
                    eprintln!(
                        "[wait] warning: could not write --out-dir error artifacts to {}: {write_err}",
                        dir.display()
                    );
                }
            }
            return Err(e);
        }
    };
    emit_outcome(outcome, args.out_dir.as_deref(), &address)
}

#[derive(Debug, Clone)]
struct WaitLoopConfig {
    store_key: String,
    session_id: String,
    address: String,
    timeout_ms: Option<u64>,
    min_attention: Option<String>,
    hang_ms: u64,
    reconnect_grace_ms: u64,
    waiter_pid: u32,
    waiter_start_time: Option<u64>,
}

#[derive(Debug)]
enum WaitTerminal {
    Response(Response),
    DaemonGone(String),
    DaemonHung(String),
}

#[async_trait(?Send)]
trait WaitClient {
    async fn request(&mut self, request: Request) -> crate::daemon::Result<Response>;
}

#[async_trait(?Send)]
trait WaitConnector {
    async fn connect_or_spawn(
        &mut self,
        store_key: &str,
    ) -> crate::daemon::Result<Box<dyn WaitClient>>;
}

struct RealWaitConnector;

#[async_trait(?Send)]
impl WaitClient for crate::daemon::DaemonClient {
    async fn request(&mut self, request: Request) -> crate::daemon::Result<Response> {
        crate::daemon::DaemonClient::request(self, &request).await
    }
}

#[async_trait(?Send)]
impl WaitConnector for RealWaitConnector {
    async fn connect_or_spawn(
        &mut self,
        store_key: &str,
    ) -> crate::daemon::Result<Box<dyn WaitClient>> {
        crate::daemon::connect_existing(store_key)
            .await
            .map(|client| Box::new(client) as Box<dyn WaitClient>)
    }
}

async fn wait_loop<C: WaitConnector>(
    connector: &mut C,
    cfg: &WaitLoopConfig,
) -> Result<WaitTerminal> {
    let wait_deadline = cfg
        .timeout_ms
        .map(|ms| tokio::time::Instant::now() + Duration::from_millis(ms));
    let mut reconnect_deadline = None;
    let mut allow_reattach = true;
    let mut retried_after_attach = false;
    let mut last_reconnect_error = None;

    loop {
        let timeout_ms = remaining_wait_timeout_ms(wait_deadline);
        if matches!(timeout_ms, Some(0)) {
            return Ok(WaitTerminal::Response(Response::Timeout));
        }
        let mut client = match reconnect_deadline {
            Some(deadline) => {
                match connect_within_grace(connector, &cfg.store_key, deadline).await? {
                    Some(client) => client,
                    None => {
                        return Ok(WaitTerminal::DaemonGone(
                            last_reconnect_error
                                .unwrap_or_else(|| "reconnect grace expired".to_string()),
                        ))
                    }
                }
            }
            None => match connector.connect_or_spawn(&cfg.store_key).await {
                Ok(client) => client,
                Err(crate::daemon::DaemonError::Timeout(e)) => {
                    return Ok(WaitTerminal::DaemonHung(e));
                }
                Err(crate::daemon::DaemonError::NotRunning(e)) => {
                    return Ok(WaitTerminal::DaemonGone(e));
                }
                Err(crate::daemon::DaemonError::Unauthorized(e)) => {
                    return Err(crate::daemon::DaemonError::Unauthorized(e).into());
                }
                Err(crate::daemon::DaemonError::Incompatible(e)) => {
                    return Err(crate::daemon::DaemonError::Incompatible(e).into());
                }
                Err(e) => {
                    return Ok(WaitTerminal::DaemonGone(e.to_string()));
                }
            },
        };

        let request = wait_request(cfg, timeout_ms);
        let response_result = match timeout_ms {
            Some(wait_ms) => {
                let watchdog_ms = wait_ms.saturating_add(cfg.hang_ms.max(1));
                match tokio::time::timeout(
                    Duration::from_millis(watchdog_ms),
                    client.request(request),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => {
                        return Ok(WaitTerminal::DaemonHung(format!(
                            "no daemon frame within timeout-ms + hang-ms ({} + {}) ms",
                            wait_ms, cfg.hang_ms
                        )));
                    }
                }
            }
            None => client.request(request).await,
        };

        let response = match response_result {
            Ok(response) => response,
            Err(e) => {
                last_reconnect_error = Some(format!("request failed: {e}"));
                begin_reconnect(
                    cfg,
                    &mut reconnect_deadline,
                    &mut allow_reattach,
                    &mut retried_after_attach,
                );
                continue;
            }
        };

        match response {
            Response::Error { code, message, .. }
                if code == crate::daemon_ipc::ERROR_NOT_RUNNING =>
            {
                last_reconnect_error = Some(format!("{code}: {message}"));
                begin_reconnect(
                    cfg,
                    &mut reconnect_deadline,
                    &mut allow_reattach,
                    &mut retried_after_attach,
                );
            }
            Response::Error {
                code,
                message,
                needs_attach_reason,
            } if code == ERROR_NEEDS_ATTACH => {
                if needs_attach_reason == Some(NeedsAttachReason::DeliberatelyDetached) {
                    return Err(anyhow!("{code}: {message}"));
                }
                if !allow_reattach || retried_after_attach {
                    return Err(anyhow!("{code}: {message}"));
                }
                let deadline = *reconnect_deadline.get_or_insert_with(|| {
                    tokio::time::Instant::now() + Duration::from_millis(cfg.reconnect_grace_ms)
                });
                match register_for_retry(connector, cfg, deadline).await? {
                    Some(()) => retried_after_attach = true,
                    None => {
                        return Ok(WaitTerminal::DaemonGone(
                            "reconnect grace expired before re-register completed".to_string(),
                        ));
                    }
                }
            }
            Response::Message { .. } | Response::Timeout | Response::PresenceEnded => {
                return Ok(WaitTerminal::Response(response));
            }
            Response::Error { code, message, .. } => return Err(anyhow!("{code}: {message}")),
            other => return Err(anyhow!("unexpected daemon wait response: {other:?}")),
        }
    }
}

fn wait_request(cfg: &WaitLoopConfig, timeout_ms: Option<u64>) -> Request {
    Request::Wait {
        store_key: cfg.store_key.clone(),
        session_id: cfg.session_id.clone(),
        address: cfg.address.clone(),
        attention: None,
        min_attention: cfg.min_attention.clone(),
        timeout_ms,
        waiter_pid: Some(cfg.waiter_pid),
        waiter_start_time: cfg.waiter_start_time,
    }
}

fn remaining_wait_timeout_ms(deadline: Option<tokio::time::Instant>) -> Option<u64> {
    deadline.map(|deadline| {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            0
        } else {
            deadline
                .duration_since(now)
                .as_millis()
                .min(u128::from(u64::MAX)) as u64
        }
    })
}

fn begin_reconnect(
    cfg: &WaitLoopConfig,
    reconnect_deadline: &mut Option<tokio::time::Instant>,
    allow_reattach: &mut bool,
    retried_after_attach: &mut bool,
) {
    if reconnect_deadline.is_none() {
        *reconnect_deadline =
            Some(tokio::time::Instant::now() + Duration::from_millis(cfg.reconnect_grace_ms));
    }
    *allow_reattach = true;
    *retried_after_attach = false;
}

async fn connect_within_grace<C: WaitConnector>(
    connector: &mut C,
    store_key: &str,
    deadline: tokio::time::Instant,
) -> Result<Option<Box<dyn WaitClient>>> {
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        let remaining = deadline.duration_since(now);
        match tokio::time::timeout(remaining, connector.connect_or_spawn(store_key)).await {
            Ok(Ok(client)) => return Ok(Some(client)),
            Ok(Err(e @ crate::daemon::DaemonError::Incompatible(_))) => return Err(e.into()),
            Ok(Err(crate::daemon::DaemonError::Unauthorized(_))) => {
                tokio::time::sleep(
                    Duration::from_millis(RECONNECT_RETRY_SLEEP_MS)
                        .min(deadline.saturating_duration_since(tokio::time::Instant::now())),
                )
                .await;
            }
            Ok(Err(_)) | Err(_) => {
                tokio::time::sleep(
                    Duration::from_millis(RECONNECT_RETRY_SLEEP_MS)
                        .min(deadline.saturating_duration_since(tokio::time::Instant::now())),
                )
                .await;
            }
        }
    }
}

async fn register_for_retry<C: WaitConnector>(
    connector: &mut C,
    cfg: &WaitLoopConfig,
    deadline: tokio::time::Instant,
) -> Result<Option<()>> {
    loop {
        let mut client = match connect_within_grace(connector, &cfg.store_key, deadline).await? {
            Some(client) => client,
            None => return Ok(None),
        };
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        let response = tokio::time::timeout(
            deadline.duration_since(now),
            client.request(Request::Register {
                store_key: cfg.store_key.clone(),
                address: cfg.address.clone(),
                session_id: cfg.session_id.clone(),
                occupant: default_occupant(),
                description: None,
                scope: None,
                tags: None,
                watch_pids: Vec::new(),
                recovery: true,
            }),
        )
        .await;
        match response {
            Ok(Ok(Response::Registered { .. })) => return Ok(Some(())),
            Ok(Ok(Response::Error { code, .. }))
                if code == crate::daemon_ipc::ERROR_NOT_RUNNING =>
            {
                tokio::time::sleep(Duration::from_millis(RECONNECT_RETRY_SLEEP_MS)).await;
            }
            Ok(Ok(Response::Error { code, message, .. })) => {
                return Err(anyhow!("{code}: {message}"));
            }
            Ok(Ok(other)) => return Err(anyhow!("unexpected daemon register response: {other:?}")),
            Ok(Err(_)) | Err(_) => {
                tokio::time::sleep(Duration::from_millis(RECONNECT_RETRY_SLEEP_MS)).await;
            }
        }
    }
}

fn reconnect_grace_ms(arg: Option<u64>) -> u64 {
    arg.or_else(|| {
        std::env::var("TELEX_RECONNECT_GRACE_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
    })
    .unwrap_or(DEFAULT_RECONNECT_GRACE_MS)
}

/// The terminal result of a `wait`, decoupled from how it is reported so the same outcome can
/// be both printed (stdout/stderr) and persisted to `--out-dir` artifacts.
struct WaitOutcome {
    exit_code: i32,
    outcome: &'static str,
    detail: Option<String>,
    message: Option<serde_json::Value>,
}

impl WaitOutcome {
    fn daemon_gone(detail: String) -> Self {
        WaitOutcome {
            exit_code: 3,
            outcome: "daemon-gone",
            detail: Some(detail),
            message: None,
        }
    }

    fn daemon_hung(detail: String) -> Self {
        WaitOutcome {
            exit_code: 4,
            outcome: "daemon-hung",
            detail: Some(detail),
            message: None,
        }
    }

    fn error(detail: String) -> Self {
        WaitOutcome {
            exit_code: 1,
            outcome: "error",
            detail: Some(detail),
            message: None,
        }
    }

    fn from_response(response: Response) -> Result<Self> {
        match response {
            Response::Message {
                id,
                thread_id,
                parent_id,
                from_addr,
                to_addr,
                delivered_to,
                primary_to,
                cc,
                delivery_role,
                kind,
                attention,
                requires_disposition,
                requires_disposition_for_current_recipient,
                subject,
                body,
                sent_at_ms,
                buffered_at_ms,
                lease_epoch,
            } => {
                let waiter_exit_ms = now_ms();
                let message = serde_json::json!({
                    "id": id,
                    "thread_id": thread_id,
                    "parent_id": parent_id,
                    "from": from_addr,
                    "to": to_addr,
                    "delivered_to": delivered_to,
                    "primary_to": primary_to,
                    "cc": cc,
                    "delivery_role": delivery_role,
                    "kind": kind,
                    "attention": attention,
                    "requires_disposition": requires_disposition,
                    "requires_disposition_for_current_recipient": requires_disposition_for_current_recipient,
                    "subject": subject,
                    "body": body,
                    "sent_at_ms": sent_at_ms,
                    "buffered_at_ms": buffered_at_ms,
                    "lease_epoch": lease_epoch,
                    "waiter_exit_ms": waiter_exit_ms,
                    "backend_ms": buffered_at_ms - sent_at_ms,
                    "send_to_exit_ms": waiter_exit_ms - sent_at_ms,
                });
                Ok(WaitOutcome {
                    exit_code: 0,
                    outcome: "message",
                    detail: None,
                    message: Some(message),
                })
            }
            Response::Timeout => Ok(WaitOutcome {
                exit_code: 2,
                outcome: "idle-timeout",
                detail: None,
                message: None,
            }),
            Response::PresenceEnded => Ok(WaitOutcome {
                exit_code: 5,
                outcome: "presence-ended",
                detail: None,
                message: None,
            }),
            other => Err(anyhow!("unexpected daemon wait response: {other:?}")),
        }
    }
}

fn emit_outcome(outcome: WaitOutcome, out_dir: Option<&Path>, address: &str) -> Result<i32> {
    match &outcome.message {
        Some(message) => println!("{message}"),
        None => match outcome.outcome {
            "idle-timeout" => eprintln!("[wait] idle-timeout (no message)"),
            "presence-ended" => eprintln!("[wait] presence-ended"),
            "daemon-gone" => {
                eprintln!(
                    "[wait] daemon-gone: {}",
                    outcome.detail.as_deref().unwrap_or("")
                )
            }
            "daemon-hung" => eprintln!("[wait] HUNG: {}", outcome.detail.as_deref().unwrap_or("")),
            _ => {}
        },
    }
    if let Some(dir) = out_dir {
        if let Err(e) = write_wait_artifacts(dir, &outcome, address) {
            eprintln!(
                "[wait] warning: could not write --out-dir artifacts to {}: {e}",
                dir.display()
            );
        }
    }
    Ok(outcome.exit_code)
}

/// Persist the wait outcome so a detached, variable-free `telex wait --out-dir <DIR>` can deliver
/// both the message and the terminal outcome to a woken agent that cannot capture the detached
/// process's stdout or real exit code. `message.json` is written only on delivery; `status.json`
/// is always written; `exit.code` is written **last** as the completion marker, so a reader can
/// treat its presence as "the wait finished and all artifacts are present".
fn write_wait_artifacts(dir: &Path, outcome: &WaitOutcome, address: &str) -> std::io::Result<()> {
    ensure_out_dir(dir)?;
    let message_path = dir.join("message.json");
    let status = serde_json::json!({
        "outcome": outcome.outcome,
        "exit_code": outcome.exit_code,
        "detail": outcome.detail,
        "address": address,
        "written_at_ms": now_ms(),
    });
    if let Some(message) = &outcome.message {
        let body = serde_json::to_string_pretty(message).unwrap_or_else(|_| message.to_string());
        atomic_write(&message_path, body.as_bytes())?;
        let delivery = serde_json::json!({
            "delivered_to": message.get("delivered_to"),
            "primary_to": message.get("primary_to"),
            "cc": message.get("cc"),
            "delivery_role": message.get("delivery_role"),
            "requires_disposition_for_current_recipient": message.get("requires_disposition_for_current_recipient"),
        });
        let envelope = serde_json::json!({
            "message": message,
            "delivery": delivery,
            "status": status,
        });
        let envelope_body =
            serde_json::to_string_pretty(&envelope).unwrap_or_else(|_| envelope.to_string());
        atomic_write(&dir.join("delivery.json"), envelope_body.as_bytes())?;
    } else if message_path.exists() {
        // The out-dir may be reused across re-arms; drop any prior payload so a non-delivery
        // outcome can never leave a stale message.json that a naive reader might re-consume.
        let _ = std::fs::remove_file(&message_path);
        let _ = std::fs::remove_file(dir.join("delivery.json"));
    }
    let status_body = serde_json::to_string_pretty(&status).unwrap_or_else(|_| status.to_string());
    atomic_write(&dir.join("status.json"), status_body.as_bytes())?;
    atomic_write(
        &dir.join("exit.code"),
        format!("{}\n", outcome.exit_code).as_bytes(),
    )?;
    Ok(())
}

/// Publish the waiter process identity as soon as `wait` starts blocking. This gives runtimes that
/// hide detached-process handles (notably Copilot CLI) a first-class, non-command-line-hunting way to
/// find the waiter during teardown.
fn write_wait_start_artifacts(dir: &Path, waiter_pid: u32) -> std::io::Result<()> {
    ensure_out_dir(dir)?;
    remove_stale_wait_completion_artifacts(dir)?;
    let status = serde_json::json!({
        "outcome": "armed",
        "exit_code": null,
        "detail": null,
        "written_at_ms": now_ms(),
    });
    let status_body = serde_json::to_string_pretty(&status).unwrap_or_else(|_| status.to_string());
    atomic_write(&dir.join("status.json"), status_body.as_bytes())?;
    atomic_write(&dir.join("wait.pid"), format!("{waiter_pid}\n").as_bytes())
}

fn remove_stale_wait_completion_artifacts(dir: &Path) -> std::io::Result<()> {
    for name in ["exit.code", "message.json", "delivery.json"] {
        let path = dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Create the artifact directory owner-only. The message body is operational content, so on Unix
/// the directory is created `0700` (Windows local app data / `%TEMP%` are already per-user, and the
/// daemon's owner-private machinery is reserved for authority-bearing paths — see ADR 0025/0026).
fn ensure_out_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

/// Write `bytes` to `path` via a sibling temp file + rename so a reader never observes a
/// partially written artifact. On Unix the file is owner-only (`0600`) since `message.json` may
/// contain the message body.
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::DaemonError;
    use crate::daemon_ipc::ERROR_NOT_RUNNING;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    struct ScriptConnector {
        clients: VecDeque<ScriptClient>,
        connect_errors: VecDeque<DaemonError>,
        connects: usize,
        requests: Arc<Mutex<Vec<Request>>>,
    }

    struct ScriptClient {
        actions: VecDeque<ScriptAction>,
        requests: Arc<Mutex<Vec<Request>>>,
    }

    enum ScriptAction {
        Response(Response),
        Error(DaemonError),
        DelayResponse(Duration, Response),
    }

    #[async_trait(?Send)]
    impl WaitClient for ScriptClient {
        async fn request(&mut self, request: Request) -> crate::daemon::Result<Response> {
            self.requests.lock().unwrap().push(request);
            let action = self
                .actions
                .pop_front()
                .expect("scripted client request action");
            match action {
                ScriptAction::Response(response) => Ok(response),
                ScriptAction::Error(error) => Err(error),
                ScriptAction::DelayResponse(delay, response) => {
                    tokio::time::sleep(delay).await;
                    Ok(response)
                }
            }
        }
    }

    #[async_trait(?Send)]
    impl WaitConnector for ScriptConnector {
        async fn connect_or_spawn(
            &mut self,
            _store_key: &str,
        ) -> crate::daemon::Result<Box<dyn WaitClient>> {
            self.connects += 1;
            if let Some(client) = self.clients.pop_front() {
                return Ok(Box::new(client));
            }
            if let Some(error) = self.connect_errors.pop_front() {
                return Err(error);
            }
            panic!("scripted connect client")
        }
    }

    impl ScriptConnector {
        fn new() -> Self {
            Self {
                clients: VecDeque::new(),
                connect_errors: VecDeque::new(),
                connects: 0,
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn client(mut self, actions: Vec<ScriptAction>) -> Self {
            self.clients.push_back(ScriptClient {
                actions: actions.into(),
                requests: self.requests.clone(),
            });
            self
        }

        fn request_ops(&self) -> Vec<&'static str> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .map(|request| match request {
                    Request::Wait { .. } => "wait",
                    Request::Register { .. } => "register",
                    other => panic!("unexpected request in wait script: {other:?}"),
                })
                .collect()
        }
    }

    fn cfg() -> WaitLoopConfig {
        WaitLoopConfig {
            store_key: "sqlite:C:\\store.db".to_string(),
            session_id: "session-1".to_string(),
            address: "addr:a".to_string(),
            timeout_ms: Some(1_000),
            min_attention: None,
            hang_ms: 1_000,
            reconnect_grace_ms: 500,
            waiter_pid: std::process::id(),
            waiter_start_time: crate::session_watch::capture_process_start_time(std::process::id()),
        }
    }

    fn message_response() -> Response {
        Response::Message {
            id: 7,
            thread_id: 7,
            parent_id: None,
            from_addr: Some("addr:b".to_string()),
            to_addr: "addr:a".to_string(),
            delivered_to: "addr:a".to_string(),
            primary_to: "addr:a".to_string(),
            cc: Vec::new(),
            delivery_role: "to".to_string(),
            kind: "note".to_string(),
            attention: "background".to_string(),
            requires_disposition: false,
            requires_disposition_for_current_recipient: false,
            subject: None,
            body: "hello".to_string(),
            sent_at_ms: now_ms(),
            buffered_at_ms: now_ms(),
            lease_epoch: Some(2),
        }
    }

    #[tokio::test]
    async fn wait_reconnects_after_request_eof_and_reissues_wait() {
        let mut connector = ScriptConnector::new()
            .client(vec![ScriptAction::Error(DaemonError::Protocol(
                "daemon closed the connection".to_string(),
            ))])
            .client(vec![ScriptAction::Response(message_response())]);

        let outcome = wait_loop(&mut connector, &cfg()).await.unwrap();
        match outcome {
            WaitTerminal::Response(Response::Message { id, .. }) => assert_eq!(id, 7),
            other => panic!("expected message after reconnect, got {other:?}"),
        }

        assert_eq!(connector.connects, 2);
        assert_eq!(connector.request_ops(), vec!["wait", "wait"]);
    }

    #[tokio::test]
    async fn unbounded_idle_wait_can_outlive_hang_ms_without_hung_exit() {
        let mut cfg = cfg();
        cfg.timeout_ms = None;
        cfg.hang_ms = 1;
        let mut connector = ScriptConnector::new().client(vec![ScriptAction::DelayResponse(
            Duration::from_millis(20),
            message_response(),
        )]);

        let outcome = wait_loop(&mut connector, &cfg).await.unwrap();
        match outcome {
            WaitTerminal::Response(Response::Message { id, .. }) => assert_eq!(id, 7),
            other => panic!("expected delayed message, got {other:?}"),
        }
        assert_eq!(connector.request_ops(), vec!["wait"]);
    }

    #[tokio::test]
    async fn finite_wait_watchdog_fires_after_timeout_plus_hang_ms() {
        let mut cfg = cfg();
        cfg.timeout_ms = Some(5);
        cfg.hang_ms = 5;
        let mut connector = ScriptConnector::new().client(vec![ScriptAction::DelayResponse(
            Duration::from_millis(50),
            Response::Timeout,
        )]);

        let outcome = wait_loop(&mut connector, &cfg).await.unwrap();
        match outcome {
            WaitTerminal::DaemonHung(message) => {
                assert!(message.contains("timeout-ms + hang-ms"));
            }
            other => panic!("expected hung watchdog, got {other:?}"),
        }
        assert_eq!(connector.request_ops(), vec!["wait"]);
    }

    #[tokio::test]
    async fn needs_attach_after_reconnect_registers_then_rewaits() {
        let mut connector = ScriptConnector::new()
            .client(vec![ScriptAction::Error(DaemonError::Protocol(
                "daemon closed the connection".to_string(),
            ))])
            .client(vec![ScriptAction::Response(
                crate::daemon_ipc::needs_attach("membership lost on restart"),
            )])
            .client(vec![ScriptAction::Response(Response::Registered {
                lease_epoch: 2,
                owner_instance_id: "new-daemon".to_string(),
            })])
            .client(vec![ScriptAction::Response(message_response())]);

        let outcome = wait_loop(&mut connector, &cfg()).await.unwrap();
        match outcome {
            WaitTerminal::Response(Response::Message { id, .. }) => assert_eq!(id, 7),
            other => panic!("expected message after reattach, got {other:?}"),
        }
        assert_eq!(connector.connects, 4);
        assert_eq!(
            connector.request_ops(),
            vec!["wait", "wait", "register", "wait"]
        );
    }

    #[tokio::test]
    async fn initial_needs_attach_registers_then_rewaits() {
        let mut connector = ScriptConnector::new()
            .client(vec![ScriptAction::Response(
                crate::daemon_ipc::needs_attach("not attached"),
            )])
            .client(vec![ScriptAction::Response(Response::Registered {
                lease_epoch: 1,
                owner_instance_id: "daemon".to_string(),
            })])
            .client(vec![ScriptAction::Response(message_response())]);

        let outcome = wait_loop(&mut connector, &cfg()).await.unwrap();
        assert!(matches!(
            outcome,
            WaitTerminal::Response(Response::Message { id: 7, .. })
        ));
        assert_eq!(connector.request_ops(), vec!["wait", "register", "wait"]);
    }

    #[tokio::test]
    async fn deliberate_detach_needs_attach_is_terminal() {
        let mut connector = ScriptConnector::new().client(vec![ScriptAction::Response(
            crate::daemon_ipc::needs_attach_with_reason(
                "deliberately detached",
                NeedsAttachReason::DeliberatelyDetached,
            ),
        )]);

        let err = wait_loop(&mut connector, &cfg()).await.unwrap_err();
        assert!(err.to_string().contains(ERROR_NEEDS_ATTACH));
        assert_eq!(connector.request_ops(), vec!["wait"]);
    }

    #[tokio::test]
    async fn draining_response_enters_reconnect_grace() {
        let mut connector = ScriptConnector::new()
            .client(vec![ScriptAction::Response(
                crate::daemon_ipc::error_response(ERROR_NOT_RUNNING, "daemon is draining"),
            )])
            .client(vec![ScriptAction::Response(
                crate::daemon_ipc::needs_attach("membership lost on drain"),
            )])
            .client(vec![ScriptAction::Response(Response::Registered {
                lease_epoch: 2,
                owner_instance_id: "new-daemon".to_string(),
            })])
            .client(vec![ScriptAction::Response(message_response())]);

        let outcome = wait_loop(&mut connector, &cfg()).await.unwrap();
        assert!(matches!(
            outcome,
            WaitTerminal::Response(Response::Message { id: 7, .. })
        ));
        assert_eq!(
            connector.request_ops(),
            vec!["wait", "wait", "register", "wait"]
        );
    }

    #[tokio::test]
    async fn reconnect_grace_expiry_returns_daemon_gone_terminal() {
        let mut cfg = cfg();
        cfg.reconnect_grace_ms = 1;
        let mut connector = ScriptConnector::new().client(vec![ScriptAction::Error(
            DaemonError::Protocol("daemon closed the connection".to_string()),
        )]);
        connector
            .connect_errors
            .push_back(DaemonError::NotRunning("still down".to_string()));

        let outcome = wait_loop(&mut connector, &cfg).await.unwrap();
        assert!(matches!(outcome, WaitTerminal::DaemonGone(_)));
        assert_eq!(connector.request_ops(), vec!["wait"]);
    }

    fn artifact_dir(label: &str) -> std::path::PathBuf {
        let n = now_ms();
        std::env::temp_dir().join(format!(
            "telex-wait-artifacts-{}-{label}-{n}",
            std::process::id()
        ))
    }

    #[test]
    fn out_dir_message_writes_message_status_and_exit_code() {
        let dir = artifact_dir("msg");
        let outcome = WaitOutcome::from_response(message_response()).unwrap();
        let code = emit_outcome(outcome, Some(dir.as_path()), "addr:a").unwrap();
        assert_eq!(code, 0);

        let message: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("message.json")).unwrap())
                .unwrap();
        assert_eq!(message.get("id").and_then(|v| v.as_i64()), Some(7));
        assert_eq!(message.get("body").and_then(|v| v.as_str()), Some("hello"));

        let status: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("status.json")).unwrap())
                .unwrap();
        assert_eq!(
            status.get("outcome").and_then(|v| v.as_str()),
            Some("message")
        );
        assert_eq!(status.get("exit_code").and_then(|v| v.as_i64()), Some(0));
        assert_eq!(
            status.get("address").and_then(|v| v.as_str()),
            Some("addr:a")
        );
        let delivery: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("delivery.json")).unwrap())
                .unwrap();
        assert_eq!(
            delivery
                .get("message")
                .and_then(|m| m.get("id"))
                .and_then(|v| v.as_i64()),
            Some(7)
        );
        assert_eq!(
            delivery
                .get("delivery")
                .and_then(|d| d.get("delivery_role"))
                .and_then(|v| v.as_str()),
            Some("to")
        );
        assert_eq!(
            delivery
                .get("status")
                .and_then(|s| s.get("outcome"))
                .and_then(|v| v.as_str()),
            Some("message")
        );

        assert_eq!(
            std::fs::read_to_string(dir.join("exit.code"))
                .unwrap()
                .trim(),
            "0"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn out_dir_startup_writes_wait_pid() {
        let dir = artifact_dir("pid");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("exit.code"), b"0\n").unwrap();
        std::fs::write(dir.join("status.json"), b"stale").unwrap();
        std::fs::write(dir.join("message.json"), b"stale").unwrap();
        std::fs::write(dir.join("delivery.json"), b"stale").unwrap();
        write_wait_start_artifacts(&dir, 12_345).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("wait.pid"))
                .unwrap()
                .trim(),
            "12345"
        );
        assert!(
            !dir.join("exit.code").exists(),
            "startup must clear stale completion marker"
        );
        let status: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("status.json")).unwrap())
                .unwrap();
        assert_eq!(status.get("outcome").and_then(|v| v.as_str()), Some("armed"));
        assert!(status.get("exit_code").unwrap().is_null());
        assert!(!dir.join("message.json").exists());
        assert!(!dir.join("delivery.json").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn out_dir_timeout_writes_status_and_exit_code_without_message() {
        let dir = artifact_dir("timeout");
        let outcome = WaitOutcome::from_response(Response::Timeout).unwrap();
        let code = emit_outcome(outcome, Some(dir.as_path()), "addr:a").unwrap();
        assert_eq!(code, 2);

        assert!(!dir.join("message.json").exists());
        let status: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("status.json")).unwrap())
                .unwrap();
        assert_eq!(
            status.get("outcome").and_then(|v| v.as_str()),
            Some("idle-timeout")
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("exit.code"))
                .unwrap()
                .trim(),
            "2"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn out_dir_daemon_gone_records_detail_in_status() {
        let dir = artifact_dir("gone");
        let outcome = WaitOutcome::daemon_gone("reconnect grace expired".to_string());
        let code = emit_outcome(outcome, Some(dir.as_path()), "addr:a").unwrap();
        assert_eq!(code, 3);

        let status: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("status.json")).unwrap())
                .unwrap();
        assert_eq!(
            status.get("outcome").and_then(|v| v.as_str()),
            Some("daemon-gone")
        );
        assert_eq!(
            status.get("detail").and_then(|v| v.as_str()),
            Some("reconnect grace expired")
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("exit.code"))
                .unwrap()
                .trim(),
            "3"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn out_dir_error_records_exit_one_and_detail() {
        let dir = artifact_dir("error");
        write_wait_artifacts(
            &dir,
            &WaitOutcome::error("Incompatible: stale daemon".to_string()),
            "addr:a",
        )
        .unwrap();
        let status: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("status.json")).unwrap())
                .unwrap();
        assert_eq!(
            status.get("outcome").and_then(|v| v.as_str()),
            Some("error")
        );
        assert_eq!(status.get("exit_code").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(
            status.get("detail").and_then(|v| v.as_str()),
            Some("Incompatible: stale daemon")
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("exit.code"))
                .unwrap()
                .trim(),
            "1"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn out_dir_is_created_when_missing() {
        let dir = artifact_dir("nested").join("a").join("b");
        let outcome = WaitOutcome::from_response(Response::Timeout).unwrap();
        emit_outcome(outcome, Some(dir.as_path()), "addr:a").unwrap();
        assert!(dir.join("exit.code").exists());
        std::fs::remove_dir_all(dir.parent().unwrap().parent().unwrap()).ok();
    }

    #[test]
    fn out_dir_reuse_clears_stale_message_on_non_delivery() {
        let dir = artifact_dir("reuse");
        emit_outcome(
            WaitOutcome::from_response(message_response()).unwrap(),
            Some(dir.as_path()),
            "addr:a",
        )
        .unwrap();
        assert!(dir.join("message.json").exists());
        assert!(dir.join("delivery.json").exists());

        // Re-arm into the same dir, this time with no delivery: the stale payload must be gone.
        emit_outcome(
            WaitOutcome::from_response(Response::Timeout).unwrap(),
            Some(dir.as_path()),
            "addr:a",
        )
        .unwrap();
        assert!(!dir.join("message.json").exists());
        assert!(!dir.join("delivery.json").exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("exit.code"))
                .unwrap()
                .trim(),
            "2"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn out_dir_artifacts_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = artifact_dir("perms");
        emit_outcome(
            WaitOutcome::from_response(message_response()).unwrap(),
            Some(dir.as_path()),
            "addr:a",
        )
        .unwrap();
        let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700, "out-dir should be owner-only");
        for name in ["message.json", "status.json", "exit.code"] {
            let mode = std::fs::metadata(dir.join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "{name} should be owner-only");
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
