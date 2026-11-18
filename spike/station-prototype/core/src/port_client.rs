//! Application Client port: the intended production seam. Uses only the public
//! `telex::application_client` surface (no CLI parsing, no daemon IPC), except
//! for counterpart presence, which the shared client does not expose yet.

use crate::model::*;
use crate::port::{PortError, Reconciled, Sent, TelexPort};
use crate::port_cli::presence_from_directory;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use telex::application_client::{
    AckHandle, AddressLifecycleResult, AddressSpec, ApplicationCapability, ApplicationClient,
    ApplicationClientConfig, ApplicationClientError, ApplicationDaemonBootstrap,
    ApplicationResponsibility, EvidenceState, ExactDeliveryIdentity, LogicalStoreId, OperationId,
    OperationReconciliation, PrincipalVerification, ReceiptAxes, RecordedOperationOutcome,
    RecoveryHandle, RejectionRetryability, ReplyRequest, SendRequest, SendResult,
};

pub struct ClientPort {
    client: ApplicationClient,
    /// Optional CLI used only for the counterpart-presence prototype seam.
    cli: Option<PathBuf>,
    acks: Mutex<BTreeMap<(i64, String), AckHandle>>,
}

fn port_error(error: ApplicationClientError) -> PortError {
    let message = error.to_string();
    match error {
        ApplicationClientError::Indeterminate { .. } | ApplicationClientError::TransportUncertain(_) => {
            PortError::uncertain(message)
        }
        ApplicationClientError::MembershipLost { .. } => PortError::needs_attach(message),
        _ => PortError::new(message),
    }
}

fn split_cc(cc: Option<&str>) -> Vec<String> {
    cc.map(|c| c.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default()
}

fn receipt_from(axes: &ReceiptAxes) -> Receipt {
    Receipt {
        occupied_at_send: axes.occupied_at_acceptance,
        consumed: matches!(axes.recipient_consumption, EvidenceState::Accepted),
        disposition: match &axes.workflow_disposition {
            EvidenceState::Disposition(state) => Some(state.clone()),
            _ => None,
        },
        refreshed_at_ms: now_ms(),
    }
}

fn sent(result: &SendResult) -> Sent {
    Sent {
        message_id: result.message_id,
        thread_id: result.thread_id,
        sender: result.sender.clone(),
        recipient: result.recipient.clone(),
        occupied: result.axes.occupied_at_acceptance,
        replayed: result.replayed,
    }
}

impl ClientPort {
    pub async fn connect(
        responsibility: String,
        backend: Option<String>,
        bootstrap: ApplicationDaemonBootstrap,
        cli: Option<PathBuf>,
    ) -> Result<Self, PortError> {
        let config = ApplicationClientConfig {
            responsibility: ApplicationResponsibility(responsibility),
            backend,
            db_override: None,
        };
        let client = ApplicationClient::connect_with_daemon(config, bootstrap).await.map_err(port_error)?;
        Ok(ClientPort { client, cli, acks: Mutex::new(BTreeMap::new()) })
    }

    fn reply_request(item: &OutboxItem, reply_to: i64) -> ReplyRequest {
        ReplyRequest {
            operation_id: OperationId(item.operation_id.clone()),
            sender: item.sender.clone(),
            message_id: reply_to,
            cc: Vec::new(),
            kind: "reply".into(),
            attention: item.attention.clone(),
            requires_disposition: item.requires_disposition,
            subject: item.subject.clone(),
            body: item.body.clone(),
            metadata: item.metadata.clone(),
            retry_budget: 3,
        }
    }

    fn send_request(item: &OutboxItem) -> SendRequest {
        SendRequest {
            operation_id: OperationId(item.operation_id.clone()),
            sender: item.sender.clone(),
            to: item.to.clone(),
            cc: Vec::new(),
            kind: "note".into(),
            attention: item.attention.clone(),
            requires_disposition: item.requires_disposition,
            subject: item.subject.clone(),
            body: item.body.clone(),
            metadata: item.metadata.clone(),
            retry_budget: 3,
        }
    }

    fn recovery(value: &Option<Value>) -> Option<RecoveryHandle> {
        value.clone().and_then(|v| serde_json::from_value(v).ok())
    }
}

#[async_trait]
impl TelexPort for ClientPort {
    fn kind(&self) -> &'static str {
        "application-client"
    }

    fn note(&self) -> Option<String> {
        None
    }

    fn store_id(&self) -> String {
        self.client.logical_store_id().0.clone()
    }

    async fn attach(&self, address: &str, description: &str) -> Result<(), PortError> {
        let spec = AddressSpec {
            address: address.to_string(),
            capability: ApplicationCapability::Bidirectional,
            description: Some(description.to_string()),
            scope: None,
            tags: Some("station,human".into()),
        };
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let outcome = self.client.attach(std::slice::from_ref(&spec)).await;
            if outcome.ready {
                return Ok(());
            }
            let transient = matches!(
                outcome.results.get(address),
                Some(AddressLifecycleResult::Failed(ApplicationClientError::RejectedBeforeAcceptance {
                    retryability: RejectionRetryability::Transient,
                    ..
                }))
            );
            if !transient || tokio::time::Instant::now() >= deadline {
                let detail = match outcome.results.get(address) {
                    Some(AddressLifecycleResult::Failed(error)) => error.to_string(),
                    _ => outcome
                        .validation_error
                        .as_ref()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "attach did not become ready".into()),
                };
                return Err(PortError::new(detail));
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }

    async fn detach(&self, address: &str) -> Result<(), PortError> {
        // Deliberate, durable detach: a restart must not resurrect it.
        self.client.detach(address).await.map_err(port_error)
    }

    async fn receive(&self, address: &str, timeout_ms: u64) -> Result<Option<StoredMessage>, PortError> {
        let d = match self.client.receive(address, Some(timeout_ms)).await {
            Ok(Some(d)) => d,
            Ok(None) => return Ok(None),
            // Preserve and continue: later deliveries can still progress.
            Err(ApplicationClientError::DeliveryQuarantined { .. }) => return Ok(None),
            Err(error) => return Err(PortError { needs_attach: true, ..port_error(error) }),
        };
        self.acks
            .lock()
            .unwrap()
            .insert((d.delivery.message_id, d.delivery.recipient.clone()), d.ack.clone());
        Ok(Some(StoredMessage {
            id: d.delivery.message_id,
            thread_id: d.thread_id,
            parent_id: d.parent_id,
            direction: Direction::Inbound,
            from: d.from.clone(),
            to: d.primary_to.clone(),
            cc: d.cc.clone(),
            kind: d.kind.clone(),
            attention: d.attention.clone(),
            requires_disposition: d.requires_disposition,
            subject: d.subject.clone(),
            body: d.body.clone(),
            metadata: d.metadata.clone(),
            sent_at_ms: d.sent_at_ms,
            ingested_at_ms: now_ms(),
            role: if d.delivery_role == "cc" { "cc".into() } else { "primary".into() },
            station_address: d.delivery.recipient.clone(),
            delivery: Some(DeliveryRef {
                logical_store_id: d.delivery.logical_store_id.0.clone(),
                message_id: d.delivery.message_id,
                recipient: d.delivery.recipient.clone(),
                delivery_id: d.delivery.delivery_id,
            }),
            acked: false,
            disposition: None,
            receipt: None,
            recovery: None,
            source: "receive".into(),
        }))
    }

    async fn acknowledge(&self, message: &StoredMessage) -> Result<(), PortError> {
        let key = (message.id, message.station_address.clone());
        let handle = self.acks.lock().unwrap().get(&key).cloned();
        let Some(handle) = handle else {
            return Err(PortError::new("no exact-delivery ack handle for this message"));
        };
        self.client.acknowledge(&handle).await.map_err(port_error)?;
        self.acks.lock().unwrap().remove(&key);
        Ok(())
    }

    async fn history(&self, address: &str, after_id: i64) -> Result<(Vec<StoredMessage>, i64), PortError> {
        let mut cursor = after_id;
        let mut rows = Vec::new();
        loop {
            let page = self
                .client
                .history(Some(address.to_string()), false, None, None, Some(cursor), 500)
                .await
                .map_err(port_error)?;
            let count = page.len();
            for item in page {
                cursor = cursor.max(item.message.id);
                let m = item.message;
                rows.push(StoredMessage {
                    id: m.id,
                    thread_id: m.thread_id,
                    parent_id: m.parent_id,
                    direction: Direction::Inbound,
                    from: m.from_addr.clone(),
                    to: m.to_addr.clone(),
                    cc: split_cc(m.cc.as_deref()),
                    kind: m.kind.clone(),
                    attention: m.attention.clone(),
                    requires_disposition: m.requires_disposition,
                    subject: m.subject.clone(),
                    body: m.body.clone(),
                    metadata: m.metadata.clone(),
                    sent_at_ms: m.sent_at_ms,
                    ingested_at_ms: now_ms(),
                    role: if m.to_addr == address { "primary".into() } else { "cc".into() },
                    station_address: address.to_string(),
                    delivery: item.delivery.as_ref().map(|d| DeliveryRef {
                        logical_store_id: item.logical_store_id.0.clone(),
                        message_id: m.id,
                        recipient: d.recipient.clone(),
                        delivery_id: d.id,
                    }),
                    acked: item.delivery.as_ref().map_or(false, |d| d.consumed_at_ms.is_some()),
                    disposition: item.latest_disposition.clone(),
                    receipt: None,
                    recovery: None,
                    source: "history".into(),
                });
            }
            if count < 500 {
                break;
            }
        }
        Ok((rows, cursor))
    }

    async fn prepare(&self, item: &OutboxItem) -> Result<Option<Value>, PortError> {
        let handle = match item.reply_to {
            Some(id) => self.client.prepare_reply(&Self::reply_request(item, id)).await,
            None => self.client.prepare_send(&Self::send_request(item)).await,
        }
        .map_err(port_error)?;
        Ok(serde_json::to_value(handle).ok())
    }

    async fn submit(&self, item: &OutboxItem) -> Result<Sent, PortError> {
        let result = match item.reply_to {
            Some(id) => self.client.reply(Self::reply_request(item, id)).await,
            None => self.client.send(Self::send_request(item)).await,
        }
        .map_err(port_error)?;
        Ok(sent(&result))
    }

    async fn reconcile(&self, item: &OutboxItem) -> Result<Reconciled, PortError> {
        let Some(handle) = Self::recovery(&item.recovery) else {
            return Ok(Reconciled::NotRecorded);
        };
        match self.client.reconcile_operation(&handle).await.map_err(port_error)? {
            OperationReconciliation::Recorded(record) => match record.outcome {
                RecordedOperationOutcome::Accepted(result) | RecordedOperationOutcome::Duplicate(result) => {
                    Ok(Reconciled::Accepted(sent(&result)))
                }
                other => Ok(Reconciled::Unknown(format!("{other:?}").chars().take(300).collect())),
            },
            OperationReconciliation::NotRecorded(_) => Ok(Reconciled::NotRecorded),
            OperationReconciliation::RetentionBoundaryCrossed { .. } => {
                Ok(Reconciled::Unknown("operation evidence crossed a retention boundary".into()))
            }
        }
    }

    async fn disposition(&self, message: &StoredMessage, state: &str, note: Option<&str>) -> Result<bool, PortError> {
        let delivery = message
            .delivery
            .as_ref()
            .ok_or_else(|| PortError::new(format!("message {} has no exact delivery identity yet", message.id)))?;
        let identity = ExactDeliveryIdentity {
            logical_store_id: LogicalStoreId(delivery.logical_store_id.clone()),
            message_id: delivery.message_id,
            recipient: delivery.recipient.clone(),
            delivery_id: delivery.delivery_id,
        };
        self.client
            .disposition(&message.station_address, &identity, state, note)
            .await
            .map_err(port_error)?;
        Ok(true)
    }

    async fn receipt(&self, message: &StoredMessage) -> Result<Receipt, PortError> {
        let handle = Self::recovery(&message.recovery)
            .ok_or_else(|| PortError::new("no recovery handle for this message"))?;
        let axes = self.client.refresh_receipt_axes(&handle).await.map_err(port_error)?;
        Ok(receipt_from(&axes))
    }

    async fn health(&self) -> Result<Vec<HealthView>, PortError> {
        let records = self.client.health().await.map_err(port_error)?;
        Ok(records
            .into_iter()
            .map(|h| HealthView {
                address: h.address.clone(),
                registered: h.registered,
                receive_ready: h.receive_ready,
                sender_ready: h.sender_ready,
                degraded: h.degraded,
                recovering: h.recovering,
                pending_unconsumed: h.pending_unconsumed,
                inbound_actionable: h.inbound_actionable,
                principal: h.principal.principal.clone(),
                principal_verification: match h.principal.verification {
                    PrincipalVerification::Verified => "verified",
                    PrincipalVerification::Unverified => "unverified",
                    PrincipalVerification::Unavailable => "unavailable",
                }
                .into(),
                last_error: None,
                last_receive_ms: None,
                evidence: h.evidence.clone(),
            })
            .collect())
    }

    /// PROTOTYPE SEAM: the Application Client has no directory or occupancy read
    /// for *other* addresses, so counterpart presence comes from the CLI's
    /// `address list --json`. Production needs a shared-client semantic for it.
    async fn presence(&self) -> Result<BTreeMap<String, Presence>, PortError> {
        let Some(cli) = &self.cli else { return Ok(BTreeMap::new()) };
        let output = tokio::process::Command::new(cli)
            .args(["address", "list", "--json"])
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .map_err(|e| PortError::new(format!("cannot run telex: {e}")))?;
        let value: Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| PortError::new(format!("unexpected directory output: {e}")))?;
        Ok(presence_from_directory(&value).0)
    }
}
