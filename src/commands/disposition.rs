use anyhow::{anyhow, Result};
use async_trait::async_trait;

use crate::cli::{Ctx, DispArgs};
use crate::config;
use std::time::Duration;

use crate::daemon_ipc::{Request, Response, ERROR_NEEDS_ATTACH, ERROR_NOT_RUNNING};
use crate::identity::{default_occupant, resolve_session_id};
use crate::model::Disposition;
use crate::output::emit;

pub async fn ack(ctx: &Ctx, args: DispArgs) -> Result<i32> {
    let address = args
        .recipient
        .clone()
        .or_else(|| ctx.address.clone())
        .ok_or_else(|| anyhow!("ack requires --recipient or global --address"))?;
    let store_key = ctx.store_key()?;
    let session_id = resolve_session_id(args.session.as_deref())?;

    let response = ack_with_retry(&store_key, &session_id, &address, args.id).await?;
    match response {
        Response::Ack {
            delivery_outcome,
            address,
            message_id,
            lease_epoch,
            ..
        } => {
            let out = serde_json::json!({
                "state": "acknowledged",
                "message_id": message_id.unwrap_or(args.id),
                "recipient": address,
                "delivery_outcome": delivery_outcome,
                "lease_epoch": lease_epoch,
            });
            emit(ctx.fmt, &out, || {
                println!(
                    "acknowledged #{} for {} ({:?})",
                    args.id,
                    out.get("recipient").and_then(|v| v.as_str()).unwrap_or("?"),
                    delivery_outcome
                );
            });
            Ok(0)
        }
        Response::Error { code, message } => Err(anyhow!("{code}: {message}")),
        other => Err(anyhow!("unexpected daemon ack response: {other:?}")),
    }
}

struct AckLoopConfig {
    store_key: String,
    session_id: String,
    address: String,
    message_id: i64,
    reconnect_grace_ms: u64,
}

async fn ack_with_retry(
    store_key: &str,
    session_id: &str,
    address: &str,
    message_id: i64,
) -> Result<Response> {
    let cfg = AckLoopConfig {
        store_key: store_key.to_string(),
        session_id: session_id.to_string(),
        address: address.to_string(),
        message_id,
        reconnect_grace_ms: reconnect_grace_ms(),
    };
    let mut connector = RealAckConnector;
    ack_loop(&mut connector, &cfg).await
}

#[async_trait(?Send)]
trait AckClient {
    async fn request(&mut self, request: Request) -> crate::daemon::Result<Response>;
}

#[async_trait(?Send)]
trait AckConnector {
    async fn connect_or_spawn(
        &mut self,
        store_key: &str,
    ) -> crate::daemon::Result<Box<dyn AckClient>>;
}

struct RealAckConnector;

#[async_trait(?Send)]
impl AckClient for crate::daemon::DaemonClient {
    async fn request(&mut self, request: Request) -> crate::daemon::Result<Response> {
        crate::daemon::DaemonClient::request(self, &request).await
    }
}

#[async_trait(?Send)]
impl AckConnector for RealAckConnector {
    async fn connect_or_spawn(
        &mut self,
        store_key: &str,
    ) -> crate::daemon::Result<Box<dyn AckClient>> {
        crate::daemon::connect_or_spawn(store_key)
            .await
            .map(|client| Box::new(client) as Box<dyn AckClient>)
    }
}

async fn ack_loop<C: AckConnector>(connector: &mut C, cfg: &AckLoopConfig) -> Result<Response> {
    let deadline =
        tokio::time::Instant::now() + Duration::from_millis(cfg.reconnect_grace_ms.max(1));
    let mut retried_after_attach = false;
    loop {
        let mut client = connect_for_retry(connector, &cfg.store_key, deadline).await?;
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err(anyhow!("reconnect grace expired before ack completed"));
        }
        let response = tokio::time::timeout(
            deadline.duration_since(now),
            client.request(Request::Ack {
                store_key: cfg.store_key.clone(),
                session_id: cfg.session_id.clone(),
                address: cfg.address.clone(),
                message_id: cfg.message_id,
            }),
        )
        .await;
        let response = match response {
            Ok(Ok(response)) => response,
            Ok(Err(_)) | Err(_) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        match &response {
            Response::Error { code, message }
                if code == ERROR_NEEDS_ATTACH
                    && !retried_after_attach
                    && !message.contains("definitely ended") =>
            {
                register_for_retry(connector, cfg, deadline).await?;
                retried_after_attach = true;
            }
            Response::Error { code, .. } if code == ERROR_NOT_RUNNING => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            _ => return Ok(response),
        }
    }
}

async fn connect_for_retry<C: AckConnector>(
    connector: &mut C,
    store_key: &str,
    deadline: tokio::time::Instant,
) -> Result<Box<dyn AckClient>> {
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!("reconnect grace expired before ack connected"));
        }
        match connector.connect_or_spawn(store_key).await {
            Ok(client) => return Ok(client),
            Err(crate::daemon::DaemonError::Incompatible(e)) => {
                return Err(anyhow!("Incompatible: {e}"));
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}

async fn register_for_retry<C: AckConnector>(
    connector: &mut C,
    cfg: &AckLoopConfig,
    deadline: tokio::time::Instant,
) -> Result<()> {
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "reconnect grace expired before ack re-attach completed"
            ));
        }
        let mut client = connect_for_retry(connector, &cfg.store_key, deadline).await?;
        let response = client
            .request(Request::Register {
                store_key: cfg.store_key.clone(),
                address: cfg.address.clone(),
                session_id: cfg.session_id.clone(),
                occupant: default_occupant(),
                description: None,
                scope: None,
                tags: None,
                watch_pids: Vec::new(),
            })
            .await;
        match response {
            Ok(Response::Registered { .. }) => return Ok(()),
            Ok(Response::Error { code, .. }) if code == ERROR_NOT_RUNNING => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(Response::Error { code, message }) => return Err(anyhow!("{code}: {message}")),
            Ok(other) => return Err(anyhow!("unexpected daemon register response: {other:?}")),
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}

fn reconnect_grace_ms() -> u64 {
    std::env::var("TELEX_RECONNECT_GRACE_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(3_000)
}

/// Apply a disposition (`state` is one of the canonical disposition strings) to a message.
pub async fn run(ctx: &Ctx, state: &str, args: DispArgs) -> Result<i32> {
    // Validate the state up front so a bad call fails clearly.
    let _ = Disposition::parse(state)?;

    let backend = ctx.backend().await?;
    let msg = backend
        .get_message(args.id)
        .await?
        .ok_or_else(|| anyhow!("message {} not found", args.id))?;

    let recipient = args
        .recipient
        .clone()
        .unwrap_or_else(|| msg.to_addr.clone());
    let by = config::principal();
    let row = backend
        .insert_disposition(args.id, &recipient, state, args.note.as_deref(), Some(&by))
        .await?;

    emit(ctx.fmt, &row, || {
        println!(
            "{} #{} by {} ({}){}",
            row.state,
            row.message_id,
            row.by_principal.as_deref().unwrap_or("?"),
            row.recipient,
            row.note
                .as_ref()
                .map(|n| format!(": {n}"))
                .unwrap_or_default()
        );
    });
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::DaemonError;
    use crate::model::DeliveryOutcome;
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
    impl AckClient for ScriptClient {
        async fn request(&mut self, request: Request) -> crate::daemon::Result<Response> {
            self.requests.lock().unwrap().push(request);
            match self.actions.pop_front().expect("script action") {
                ScriptAction::Response(response) => Ok(response),
                ScriptAction::Error(error) => Err(error),
            }
        }
    }

    #[async_trait(?Send)]
    impl AckConnector for ScriptConnector {
        async fn connect_or_spawn(
            &mut self,
            _store_key: &str,
        ) -> crate::daemon::Result<Box<dyn AckClient>> {
            self.connects += 1;
            if let Some(error) = self.connect_errors.pop_front() {
                return Err(error);
            }
            Ok(Box::new(
                self.clients.pop_front().expect("scripted ack client"),
            ))
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

        fn connect_error(mut self, error: DaemonError) -> Self {
            self.connect_errors.push_back(error);
            self
        }

        fn request_ops(&self) -> Vec<&'static str> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .map(|request| match request {
                    Request::Ack { .. } => "ack",
                    Request::Register { .. } => "register",
                    other => panic!("unexpected request in ack script: {other:?}"),
                })
                .collect()
        }
    }

    fn cfg() -> AckLoopConfig {
        AckLoopConfig {
            store_key: "sqlite:C:\\store.db".to_string(),
            session_id: "session-1".to_string(),
            address: "addr:a".to_string(),
            message_id: 7,
            reconnect_grace_ms: 500,
        }
    }

    fn ack_marked() -> Response {
        Response::Ack {
            message: Some("ack".to_string()),
            delivery_outcome: Some(DeliveryOutcome::Marked),
            address: Some("addr:a".to_string()),
            message_id: Some(7),
            lease_epoch: Some(2),
        }
    }

    #[tokio::test]
    async fn ack_reconnects_registers_and_retries_after_restart_lost_membership() {
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
            .client(vec![ScriptAction::Response(ack_marked())]);

        let response = ack_loop(&mut connector, &cfg()).await.unwrap();
        assert!(matches!(
            response,
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::Marked),
                ..
            }
        ));
        assert_eq!(
            connector.request_ops(),
            vec!["ack", "ack", "register", "ack"]
        );
    }

    #[tokio::test]
    async fn ack_does_not_reattach_after_definite_detach() {
        let mut connector = ScriptConnector::new().client(vec![ScriptAction::Response(
            crate::daemon_ipc::needs_attach("session s1 was definitely ended by Detach"),
        )]);

        let response = ack_loop(&mut connector, &cfg()).await.unwrap();
        assert!(matches!(
            response,
            Response::Error { ref code, .. } if code == ERROR_NEEDS_ATTACH
        ));
        assert_eq!(connector.request_ops(), vec!["ack"]);
    }

    #[tokio::test]
    async fn ack_retries_stale_cap_unauthorized_inside_grace() {
        let mut connector = ScriptConnector::new()
            .connect_error(DaemonError::Unauthorized(
                "server pid/start-time did not match stale cap".to_string(),
            ))
            .client(vec![ScriptAction::Response(ack_marked())]);

        let response = ack_loop(&mut connector, &cfg()).await.unwrap();
        assert!(matches!(
            response,
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::Marked),
                ..
            }
        ));
        assert_eq!(connector.connects, 2);
        assert_eq!(connector.request_ops(), vec!["ack"]);
    }
}
