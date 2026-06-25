use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::time::Duration;

use crate::cli::{Ctx, WaitArgs};
use crate::daemon_ipc::{Request, Response, ERROR_NEEDS_ATTACH};
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

    let cfg = WaitLoopConfig {
        store_key,
        session_id,
        address,
        timeout_ms: args.timeout_ms,
        hang_ms: args.hang_ms,
        reconnect_grace_ms: reconnect_grace_ms(args.reconnect_grace_ms),
    };
    let mut connector = RealWaitConnector;
    match wait_loop(&mut connector, &cfg).await? {
        WaitTerminal::DaemonGone(message) => {
            eprintln!("[wait] daemon-gone: {message}");
            Ok(3)
        }
        WaitTerminal::DaemonHung(message) => {
            eprintln!("[wait] HUNG: {message}");
            Ok(4)
        }
        WaitTerminal::Response(response) => emit_response(response),
    }
}

#[derive(Debug, Clone)]
struct WaitLoopConfig {
    store_key: String,
    session_id: String,
    address: String,
    timeout_ms: Option<u64>,
    hang_ms: u64,
    reconnect_grace_ms: u64,
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
        crate::daemon::connect_or_spawn(store_key)
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
                Err(e) => return Err(e.into()),
            },
        };

        let request = wait_request(cfg, timeout_ms);
        let response = match tokio::time::timeout(
            Duration::from_millis(cfg.hang_ms.max(1)),
            client.request(request),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(e)) => {
                last_reconnect_error = Some(format!("request failed: {e}"));
                begin_reconnect(
                    cfg,
                    &mut reconnect_deadline,
                    &mut allow_reattach,
                    &mut retried_after_attach,
                );
                continue;
            }
            Err(_) => {
                return Ok(WaitTerminal::DaemonHung(format!(
                    "no daemon frame within {} ms",
                    cfg.hang_ms
                )));
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
            Response::Error { code, message, .. } if code == ERROR_NEEDS_ATTACH => {
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
        timeout_ms,
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

fn emit_response(response: Response) -> Result<i32> {
    match response {
        Response::Message {
            id,
            thread_id,
            parent_id,
            from_addr,
            to_addr,
            kind,
            attention,
            requires_disposition,
            subject,
            body,
            sent_at_ms,
            buffered_at_ms,
            lease_epoch,
        } => {
            let waiter_exit_ms = now_ms();
            println!(
                "{}",
                serde_json::json!({
                    "id": id,
                    "thread_id": thread_id,
                    "parent_id": parent_id,
                    "from": from_addr,
                    "to": to_addr,
                    "kind": kind,
                    "attention": attention,
                    "requires_disposition": requires_disposition,
                    "subject": subject,
                    "body": body,
                    "sent_at_ms": sent_at_ms,
                    "buffered_at_ms": buffered_at_ms,
                    "lease_epoch": lease_epoch,
                    "waiter_exit_ms": waiter_exit_ms,
                    "backend_ms": buffered_at_ms - sent_at_ms,
                    "send_to_exit_ms": waiter_exit_ms - sent_at_ms,
                })
            );
            Ok(0)
        }
        Response::Timeout => {
            eprintln!("[wait] idle-timeout (no message)");
            Ok(2)
        }
        Response::PresenceEnded => {
            eprintln!("[wait] presence-ended");
            Ok(5)
        }
        other => Err(anyhow!("unexpected daemon wait response: {other:?}")),
    }
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
            hang_ms: 1_000,
            reconnect_grace_ms: 500,
        }
    }

    fn message_response() -> Response {
        Response::Message {
            id: 7,
            thread_id: 7,
            parent_id: None,
            from_addr: Some("addr:b".to_string()),
            to_addr: "addr:a".to_string(),
            kind: "note".to_string(),
            attention: "background".to_string(),
            requires_disposition: false,
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
}
