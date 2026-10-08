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
        wake_on_cc: args.wake_on_cc,
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
    wake_on_cc: bool,
    hang_ms: u64,
    reconnect_grace_ms: u64,
    waiter_pid: u32,
    waiter_start_time: Option<u64>,
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
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
            Response::Message { .. }
            | Response::DeliveryQuarantined { .. }
            | Response::Timeout
            | Response::PresenceEnded => {
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
        wake_on_cc: cfg.wake_on_cc,
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
                replace_watch_pids: false,
                recovery: true,
                on_deliver: None,
                replace_on_deliver: false,
                on_deliver_wake_on_cc: false,
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
    quarantine: Option<serde_json::Value>,
}

impl WaitOutcome {
    fn daemon_gone(detail: String) -> Self {
        WaitOutcome {
            exit_code: 3,
            outcome: "daemon-gone",
            detail: Some(detail),
            message: None,
            quarantine: None,
        }
    }

    fn daemon_hung(detail: String) -> Self {
        WaitOutcome {
            exit_code: 4,
            outcome: "daemon-hung",
            detail: Some(detail),
            message: None,
            quarantine: None,
        }
    }

    fn error(detail: String) -> Self {
        WaitOutcome {
            exit_code: 1,
            outcome: "error",
            detail: Some(detail),
            message: None,
            quarantine: None,
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
                metadata,
                sent_at_ms,
                buffered_at_ms,
                delivery_id,
                snapshot_version,
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
                    "metadata": metadata,
                    "sent_at_ms": sent_at_ms,
                    "buffered_at_ms": buffered_at_ms,
                    "delivery_id": delivery_id,
                    "snapshot_version": snapshot_version,
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
                    quarantine: None,
                })
            }
            Response::Timeout => Ok(WaitOutcome {
                exit_code: 2,
                outcome: "idle-timeout",
                detail: None,
                message: None,
                quarantine: None,
            }),
            Response::PresenceEnded => Ok(WaitOutcome {
                exit_code: 5,
                outcome: "presence-ended",
                detail: None,
                message: None,
                quarantine: None,
            }),
            Response::DeliveryQuarantined {
                message_id,
                recipient,
                serialized_bytes,
                max_bytes,
                may_continue,
            } => Ok(WaitOutcome {
                exit_code: 6,
                outcome: "delivery-quarantined",
                detail: None,
                message: None,
                quarantine: Some(serde_json::json!({
                    "message_id": message_id,
                    "recipient": recipient,
                    "serialized_bytes": serialized_bytes,
                    "max_bytes": max_bytes,
                    "may_continue": may_continue,
                })),
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
            "delivery-quarantined" => {
                if let Some(quarantine) = &outcome.quarantine {
                    eprintln!("[wait] delivery-quarantined: {quarantine}");
                }
            }
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
        "quarantine": outcome.quarantine,
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
    } else {
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

pub(crate) fn write_terminal_error_artifacts(
    dir: &Path,
    address: &str,
    detail: impl Into<String>,
) -> std::io::Result<()> {
    write_wait_artifacts(dir, &WaitOutcome::error(detail.into()), address)
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
