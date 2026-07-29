//! Supported semantic client for long-lived Telex applications.
//!
//! This module is the public application boundary. Daemon frames, backend store
//! keys, paths, connection strings, and backend-specific errors stay private.

use crate::backend::Backend;
use crate::daemon_ipc::{
    DeliveryMode, MemberStatus, NeedsAttachReason, Request, Response, StationCapability,
};
use crate::model::{
    now_ms, ApplicationOperationBegin, ApplicationOperationRecord, ApplicationRecordScope,
    ApplicationStorageStats, CleanupReport, CompoundStepRecord, DeliveryOutcome, HistoryOrder,
    HistoryQuery, NewApplicationOperation, NewCompoundStepRecord, RetentionPolicy,
    StateDeltaRecord,
};
use crate::profiles::BackendProfile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex};

const STORE_ID_DOMAIN: &[u8] = b"telex-logical-store-v1\0";
const PAYLOAD_FINGERPRINT_DOMAIN: &[u8] = b"telex-application-operation-v1\0";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ApplicationResponsibility(pub String);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuntimeId(pub String);

impl RuntimeId {
    pub fn fresh() -> Result<Self, ApplicationClientError> {
        let mut bytes = [0u8; 16];
        getrandom::getrandom(&mut bytes)
            .map_err(|e| ApplicationClientError::Unavailable(e.to_string()))?;
        Ok(Self(format!("rt-{}", hex(&bytes))))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LogicalStoreId(pub String);

impl LogicalStoreId {
    fn derive(store_key: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(STORE_ID_DOMAIN);
        hasher.update(store_key.as_bytes());
        Self(format!("store-v1-{}", hex(&hasher.finalize())))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OperationId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MembershipLossReason {
    DaemonRestart,
    PredicateDeath,
    Collision,
    DeliberateDetach,
    NeedsAttach,
    OwnerDemoted,
    Unknown { raw_reason: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollisionEvidence {
    pub address: String,
    pub owner_instance_id: Option<String>,
    pub lease_epoch: Option<i64>,
    pub guidance: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplicationClientError {
    MembershipLost {
        address: String,
        reason: MembershipLossReason,
        detail: String,
    },
    Collision(CollisionEvidence),
    AmbiguousSender(Vec<String>),
    UnsupportedCapability(String),
    DeliveryMismatch {
        message_id: i64,
        recipient: String,
        delivery_id: i64,
    },
    OperationMismatch {
        operation_id: OperationId,
    },
    ResyncRequired {
        expected_version: i64,
        observed_version: i64,
    },
    Partial(String),
    Indeterminate {
        detail: String,
        recovery: RecoveryHandle,
    },
    InvalidRequest(String),
    Unavailable(String),
}

impl fmt::Display for ApplicationClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MembershipLost {
                address, reason, ..
            } => {
                write!(f, "membership lost for {address}: {reason:?}")
            }
            Self::Collision(e) => write!(f, "membership collision at {}", e.address),
            Self::AmbiguousSender(values) => write!(f, "ambiguous sender: {}", values.join(", ")),
            Self::UnsupportedCapability(value) => write!(f, "unsupported capability: {value}"),
            Self::DeliveryMismatch { message_id, .. } => {
                write!(f, "exact delivery mismatch for message {message_id}")
            }
            Self::OperationMismatch { operation_id } => {
                write!(
                    f,
                    "operation {} was reused with different input",
                    operation_id.0
                )
            }
            Self::ResyncRequired { .. } => write!(f, "state resynchronization required"),
            Self::Partial(detail) => write!(f, "partial application operation: {detail}"),
            Self::Indeterminate { detail, .. } => write!(f, "indeterminate operation: {detail}"),
            Self::InvalidRequest(detail) => write!(f, "invalid application request: {detail}"),
            Self::Unavailable(detail) => write!(f, "application client unavailable: {detail}"),
        }
    }
}

impl std::error::Error for ApplicationClientError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryPolicy {
    Strict,
    BoundedRepair { retries: u32 },
}

#[derive(Clone, Debug)]
pub struct ApplicationClientConfig {
    pub responsibility: ApplicationResponsibility,
    pub backend: Option<String>,
    pub db_override: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressSpec {
    pub address: String,
    pub capability: StationCapability,
    pub description: Option<String>,
    pub scope: Option<String>,
    pub tags: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipHandle {
    pub logical_store_id: LogicalStoreId,
    pub responsibility: ApplicationResponsibility,
    pub runtime_id: RuntimeId,
    pub address: String,
    pub capability: StationCapability,
    pub lease_epoch: i64,
    pub owner_instance_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompensationHandle {
    pub address: String,
    pub runtime_id: RuntimeId,
    pub action: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AddressLifecycleResult {
    Attached(MembershipHandle),
    Failed(ApplicationClientError),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiAddressOutcome {
    pub ready: bool,
    pub results: BTreeMap<String, AddressLifecycleResult>,
    pub compensation: Vec<CompensationHandle>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryHandle {
    pub logical_store_id: LogicalStoreId,
    pub responsibility: ApplicationResponsibility,
    pub operation_id: OperationId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceState {
    Unknown,
    NotAttempted,
    Accepted,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptAxes {
    pub durable_acceptance: EvidenceState,
    pub occupied_at_acceptance: Option<bool>,
    pub push_acceptance: EvidenceState,
    pub recipient_consumption: EvidenceState,
    pub workflow_disposition: EvidenceState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendRequest {
    pub operation_id: OperationId,
    pub sender: String,
    pub to: String,
    pub cc: Vec<String>,
    pub kind: String,
    pub attention: String,
    pub requires_disposition: bool,
    pub subject: Option<String>,
    pub body: String,
    pub metadata: Option<String>,
    pub retry_budget: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyRequest {
    pub operation_id: OperationId,
    pub sender: String,
    pub message_id: i64,
    pub cc: Vec<String>,
    pub kind: String,
    pub attention: String,
    pub requires_disposition: bool,
    pub subject: Option<String>,
    pub body: String,
    pub metadata: Option<String>,
    pub retry_budget: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendResult {
    pub logical_store_id: LogicalStoreId,
    pub operation_id: OperationId,
    pub message_id: i64,
    pub thread_id: i64,
    pub sender: String,
    pub recipient: String,
    pub axes: ReceiptAxes,
    pub replayed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExactDeliveryIdentity {
    pub logical_store_id: LogicalStoreId,
    pub message_id: i64,
    pub recipient: String,
    pub delivery_id: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckHandle {
    pub delivery: ExactDeliveryIdentity,
    runtime_id: RuntimeId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceivedDelivery {
    pub delivery: ExactDeliveryIdentity,
    pub thread_id: i64,
    pub parent_id: Option<i64>,
    pub from: Option<String>,
    pub primary_to: String,
    pub cc: Vec<String>,
    pub delivery_role: String,
    pub kind: String,
    pub attention: String,
    pub requires_disposition: bool,
    pub subject: Option<String>,
    pub body: String,
    pub metadata: Option<String>,
    pub sent_at_ms: i64,
    pub snapshot_version: i64,
    pub ack: AckHandle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AckResult {
    Marked,
    AlreadyConsumed,
    NoDelivery,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrincipalVerification {
    Verified,
    Unverified,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrincipalProvenance {
    pub principal: Option<String>,
    pub verification: PrincipalVerification,
    pub evidence: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicationHealth {
    pub logical_store_id: LogicalStoreId,
    pub responsibility: ApplicationResponsibility,
    pub runtime_id: RuntimeId,
    pub address: String,
    pub capability: StationCapability,
    pub registered: bool,
    pub lease_epoch: Option<i64>,
    pub owner_instance_id: Option<String>,
    pub pending_unconsumed: i64,
    pub inbound_actionable: i64,
    pub acknowledgment_pending: bool,
    pub sender_ready: bool,
    pub receive_ready: bool,
    pub attended_but_deaf: bool,
    pub recovering: bool,
    pub degraded: bool,
    pub stopped_or_unattended: bool,
    pub principal: PrincipalProvenance,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceReference {
    pub logical_store_id: LogicalStoreId,
    pub message_id: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SourceResolution {
    Authoritative(crate::model::MessageRow),
    CapturedOnly(crate::model::MessageRow),
    Mismatch,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryItem {
    pub logical_store_id: LogicalStoreId,
    pub message: crate::model::MessageRow,
    pub delivery: Option<crate::model::DeliveryRow>,
    pub latest_disposition: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeltaPage {
    pub logical_store_id: LogicalStoreId,
    pub from_version: i64,
    pub current_version: i64,
    pub deltas: Vec<StateDeltaRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompoundStep {
    pub step_id: String,
    pub position: i64,
    pub kind: String,
    pub prerequisites: Vec<String>,
    pub declaration: serde_json::Value,
}

#[derive(Clone, Debug)]
struct LocalMembership {
    handle: MembershipHandle,
    recovering: bool,
}

pub struct ApplicationClient {
    responsibility: ApplicationResponsibility,
    runtime_id: RuntimeId,
    logical_store_id: LogicalStoreId,
    store_key: String,
    profile: BackendProfile,
    backend: Arc<dyn Backend>,
    memberships: Mutex<BTreeMap<String, LocalMembership>>,
}

impl ApplicationClient {
    pub async fn connect(config: ApplicationClientConfig) -> Result<Self, ApplicationClientError> {
        let (_, profile) =
            crate::profiles::resolve(config.backend.as_deref(), config.db_override.as_deref())
                .map_err(unavailable)?;
        let store_key = crate::profiles::store_key(&profile, config.db_override.as_deref());
        let backend = crate::profiles::build(&profile, config.db_override.as_deref())
            .await
            .map_err(unavailable)?;
        Ok(Self {
            responsibility: config.responsibility,
            runtime_id: RuntimeId::fresh()?,
            logical_store_id: LogicalStoreId::derive(&store_key),
            store_key,
            profile,
            backend,
            memberships: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn responsibility(&self) -> &ApplicationResponsibility {
        &self.responsibility
    }

    pub fn runtime_id(&self) -> &RuntimeId {
        &self.runtime_id
    }

    pub fn logical_store_id(&self) -> &LogicalStoreId {
        &self.logical_store_id
    }

    pub async fn attach(&self, specs: &[AddressSpec]) -> MultiAddressOutcome {
        let mut results = BTreeMap::new();
        let mut compensation = Vec::new();
        for spec in specs {
            let result = self.attach_one(spec, false).await;
            if let Ok(handle) = &result {
                compensation.push(CompensationHandle {
                    address: handle.address.clone(),
                    runtime_id: self.runtime_id.clone(),
                    action: "detach".to_string(),
                });
                results.insert(
                    spec.address.clone(),
                    AddressLifecycleResult::Attached(handle.clone()),
                );
            } else if let Err(error) = result {
                results.insert(spec.address.clone(), AddressLifecycleResult::Failed(error));
            }
        }
        let ready = results
            .values()
            .all(|value| matches!(value, AddressLifecycleResult::Attached(_)));
        if ready {
            compensation.clear();
        }
        MultiAddressOutcome {
            ready,
            results,
            compensation,
        }
    }

    pub async fn reconcile(
        &self,
        spec: &AddressSpec,
        policy: RecoveryPolicy,
    ) -> Result<MembershipHandle, ApplicationClientError> {
        if policy == RecoveryPolicy::Strict {
            let current = self.membership(&spec.address)?;
            let status = match self
                .request(
                    Request::Status {
                        store_key: Some(self.store_key.clone()),
                        detail: false,
                        proof: None,
                    },
                    false,
                )
                .await?
            {
                Response::StatusReport { status } => status,
                other => {
                    return Err(ApplicationClientError::Unavailable(format!(
                        "unexpected status response: {other:?}"
                    )))
                }
            };
            if status.members.iter().any(|member| {
                member.session_id == self.runtime_id.0
                    && member.address == spec.address
                    && !member.idle
            }) {
                return Ok(current.handle);
            }
            return Err(ApplicationClientError::MembershipLost {
                address: spec.address.clone(),
                reason: MembershipLossReason::NeedsAttach,
                detail: "strict membership does not repair a lost attachment".to_string(),
            });
        }
        let retries = match policy {
            RecoveryPolicy::Strict => unreachable!(),
            RecoveryPolicy::BoundedRepair { retries } => retries,
        };
        let mut attempt = 0;
        loop {
            match self.attach_one(spec, true).await {
                Ok(handle) => return Ok(handle),
                Err(error @ ApplicationClientError::MembershipLost { .. }) if attempt < retries => {
                    attempt += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(50 * attempt as u64)).await;
                    let _ = error;
                }
                Err(error) => return Err(error),
            }
        }
    }

    async fn attach_one(
        &self,
        spec: &AddressSpec,
        recovery: bool,
    ) -> Result<MembershipHandle, ApplicationClientError> {
        let request = Request::ApplicationRegister {
            store_key: self.store_key.clone(),
            address: spec.address.clone(),
            session_id: self.runtime_id.0.clone(),
            occupant: format!(
                "{}:{}",
                self.responsibility.0,
                crate::identity::default_occupant()
            ),
            capability: spec.capability,
            description: spec.description.clone(),
            scope: spec.scope.clone(),
            tags: spec.tags.clone(),
            watch_pids: vec![crate::daemon_ipc::WatchPidSpec::anchor(std::process::id())],
            recovery,
        };
        match self.request(request, true).await? {
            Response::Registered {
                lease_epoch,
                owner_instance_id,
            } => {
                let handle = MembershipHandle {
                    logical_store_id: self.logical_store_id.clone(),
                    responsibility: self.responsibility.clone(),
                    runtime_id: self.runtime_id.clone(),
                    address: spec.address.clone(),
                    capability: spec.capability,
                    lease_epoch,
                    owner_instance_id,
                };
                self.memberships.lock().unwrap().insert(
                    spec.address.clone(),
                    LocalMembership {
                        handle: handle.clone(),
                        recovering: false,
                    },
                );
                Ok(handle)
            }
            Response::Error {
                code,
                message,
                needs_attach_reason,
            } => Err(self
                .registration_error(&spec.address, &code, &message, needs_attach_reason)
                .await),
            other => Err(ApplicationClientError::Unavailable(format!(
                "unexpected application register response: {other:?}"
            ))),
        }
    }

    async fn registration_error(
        &self,
        address: &str,
        code: &str,
        message: &str,
        reason: Option<NeedsAttachReason>,
    ) -> ApplicationClientError {
        if code == crate::daemon_ipc::ERROR_INCOMPATIBLE
            && message.contains("unknown or invalid request")
        {
            return ApplicationClientError::UnsupportedCapability(
                "daemon does not support the Application Client protocol extension".to_string(),
            );
        }
        if message.contains("already owned") || message.contains("already attended") {
            let lease = self.backend.get_lease(address).await.ok().flatten();
            return ApplicationClientError::Collision(CollisionEvidence {
                address: address.to_string(),
                owner_instance_id: lease.as_ref().and_then(|row| row.owner_instance_id.clone()),
                lease_epoch: lease.and_then(|row| row.lease_epoch),
                guidance: "wait for the current owner, reset with explicit authority, or retry within a bounded budget".to_string(),
            });
        }
        ApplicationClientError::MembershipLost {
            address: address.to_string(),
            reason: match reason {
                Some(NeedsAttachReason::RestartLost) => MembershipLossReason::DaemonRestart,
                Some(NeedsAttachReason::DeliberatelyDetached) => {
                    MembershipLossReason::DeliberateDetach
                }
                None if code == crate::daemon_ipc::ERROR_NEEDS_ATTACH => {
                    MembershipLossReason::NeedsAttach
                }
                None => MembershipLossReason::Unknown {
                    raw_reason: Some(code.to_string()),
                },
            },
            detail: message.replace(&self.store_key, &self.logical_store_id.0),
        }
    }

    pub async fn detach(&self, address: &str) -> Result<(), ApplicationClientError> {
        match self
            .request(
                Request::Detach {
                    store_key: self.store_key.clone(),
                    session_id: self.runtime_id.0.clone(),
                    address: address.to_string(),
                },
                false,
            )
            .await?
        {
            Response::Ack { .. } => {
                self.memberships.lock().unwrap().remove(address);
                Ok(())
            }
            Response::Error {
                code,
                message,
                needs_attach_reason,
            } => Err(self
                .registration_error(address, &code, &message, needs_attach_reason)
                .await),
            other => Err(ApplicationClientError::Unavailable(format!(
                "unexpected detach response: {other:?}"
            ))),
        }
    }

    pub async fn send(&self, request: SendRequest) -> Result<SendResult, ApplicationClientError> {
        self.require_sender(&request.sender)?;
        let fingerprint = payload_fingerprint(&request)?;
        let operation = NewApplicationOperation {
            logical_store_id: self.logical_store_id.0.clone(),
            application_responsibility: self.responsibility.0.clone(),
            operation_id: request.operation_id.0.clone(),
            operation_kind: "send".to_string(),
            sender: request.sender.clone(),
            recipients_json: serde_json::to_string(&(request.to.clone(), request.cc.clone()))
                .map_err(invalid)?,
            payload_fingerprint: fingerprint,
            retry_budget: request.retry_budget as i64,
            created_at_ms: now_ms(),
        };
        match self
            .backend
            .begin_application_operation(&operation)
            .await
            .map_err(unavailable)?
        {
            ApplicationOperationBegin::FingerprintMismatch(_) => {
                return Err(ApplicationClientError::OperationMismatch {
                    operation_id: request.operation_id,
                })
            }
            ApplicationOperationBegin::Replay(existing) if existing.state == "accepted" => {
                let mut result: SendResult =
                    serde_json::from_str(existing.result_json.as_deref().ok_or_else(|| {
                        ApplicationClientError::Unavailable(
                            "accepted operation has no durable result".to_string(),
                        )
                    })?)
                    .map_err(invalid)?;
                result.replayed = true;
                return Ok(result);
            }
            ApplicationOperationBegin::Replay(existing) if existing.state == "needs-attach" => {}
            ApplicationOperationBegin::Replay(existing) => {
                return Err(ApplicationClientError::Indeterminate {
                    detail: format!("operation remains {}", existing.state),
                    recovery: self.recovery_handle(request.operation_id),
                })
            }
            ApplicationOperationBegin::Started(_) => {}
        }

        let daemon_request = Request::Send {
            store_key: self.store_key.clone(),
            session_id: self.runtime_id.0.clone(),
            from_addr: Some(request.sender.clone()),
            to_addr: request.to.clone(),
            cc: normalize_cc(&request.cc),
            kind: request.kind.clone(),
            attention: request.attention.clone(),
            requires_disposition: request.requires_disposition,
            subject: request.subject.clone(),
            body: request.body.clone(),
            metadata: request.metadata.clone(),
        };
        let response = match self.request(daemon_request, false).await {
            Ok(response) => response,
            Err(error) => {
                let recovery = self.recovery_handle(request.operation_id.clone());
                let _ = self
                    .backend
                    .complete_application_operation(
                        &self.logical_store_id.0,
                        &self.responsibility.0,
                        &request.operation_id.0,
                        "indeterminate",
                        None,
                        Some(&serde_json::to_string(&recovery).map_err(invalid)?),
                    )
                    .await;
                return Err(ApplicationClientError::Indeterminate {
                    detail: error.to_string(),
                    recovery,
                });
            }
        };
        match response {
            Response::Sent { receipt } => {
                let result = SendResult {
                    logical_store_id: self.logical_store_id.clone(),
                    operation_id: request.operation_id.clone(),
                    message_id: receipt.id,
                    thread_id: receipt.thread_id,
                    sender: request.sender,
                    recipient: receipt.to,
                    axes: ReceiptAxes {
                        durable_acceptance: EvidenceState::Accepted,
                        occupied_at_acceptance: receipt.occupied,
                        push_acceptance: EvidenceState::Unknown,
                        recipient_consumption: EvidenceState::Unknown,
                        workflow_disposition: EvidenceState::Unknown,
                    },
                    replayed: false,
                };
                let result_json = serde_json::to_string(&result).map_err(invalid)?;
                self.backend
                    .complete_application_operation(
                        &self.logical_store_id.0,
                        &self.responsibility.0,
                        &request.operation_id.0,
                        "accepted",
                        Some(&result_json),
                        None,
                    )
                    .await
                    .map_err(unavailable)?;
                Ok(result)
            }
            Response::Error {
                code,
                message,
                needs_attach_reason,
            } => {
                let error = self
                    .registration_error(&request.sender, &code, &message, needs_attach_reason)
                    .await;
                let error_json = serde_json::to_string(&error).map_err(invalid)?;
                let state = if matches!(
                    error,
                    ApplicationClientError::MembershipLost {
                        reason: MembershipLossReason::DaemonRestart
                            | MembershipLossReason::NeedsAttach
                            | MembershipLossReason::PredicateDeath,
                        ..
                    }
                ) {
                    "needs-attach"
                } else {
                    "rejected"
                };
                self.backend
                    .complete_application_operation(
                        &self.logical_store_id.0,
                        &self.responsibility.0,
                        &request.operation_id.0,
                        state,
                        Some(&error_json),
                        None,
                    )
                    .await
                    .map_err(unavailable)?;
                Err(error)
            }
            other => Err(ApplicationClientError::Unavailable(format!(
                "unexpected send response: {other:?}"
            ))),
        }
    }

    pub async fn reply(&self, request: ReplyRequest) -> Result<SendResult, ApplicationClientError> {
        self.require_sender(&request.sender)?;
        let fingerprint = reply_fingerprint(&request)?;
        let operation = NewApplicationOperation {
            logical_store_id: self.logical_store_id.0.clone(),
            application_responsibility: self.responsibility.0.clone(),
            operation_id: request.operation_id.0.clone(),
            operation_kind: "reply".to_string(),
            sender: request.sender.clone(),
            recipients_json: serde_json::to_string(&(request.message_id, request.cc.clone()))
                .map_err(invalid)?,
            payload_fingerprint: fingerprint,
            retry_budget: request.retry_budget as i64,
            created_at_ms: now_ms(),
        };
        match self
            .backend
            .begin_application_operation(&operation)
            .await
            .map_err(unavailable)?
        {
            ApplicationOperationBegin::FingerprintMismatch(_) => {
                return Err(ApplicationClientError::OperationMismatch {
                    operation_id: request.operation_id,
                })
            }
            ApplicationOperationBegin::Replay(existing) if existing.state == "accepted" => {
                let mut result: SendResult =
                    serde_json::from_str(existing.result_json.as_deref().ok_or_else(|| {
                        ApplicationClientError::Unavailable(
                            "accepted reply operation has no durable result".to_string(),
                        )
                    })?)
                    .map_err(invalid)?;
                result.replayed = true;
                return Ok(result);
            }
            ApplicationOperationBegin::Replay(existing) if existing.state == "needs-attach" => {}
            ApplicationOperationBegin::Replay(existing) => {
                return Err(ApplicationClientError::Indeterminate {
                    detail: format!("reply operation remains {}", existing.state),
                    recovery: self.recovery_handle(request.operation_id),
                })
            }
            ApplicationOperationBegin::Started(_) => {}
        }

        let response = self
            .request(
                Request::ApplicationReply {
                    store_key: self.store_key.clone(),
                    session_id: self.runtime_id.0.clone(),
                    from_addr: request.sender.clone(),
                    message_id: request.message_id,
                    kind: request.kind,
                    attention: request.attention,
                    requires_disposition: request.requires_disposition,
                    subject: request.subject,
                    cc: normalize_cc(&request.cc),
                    body: request.body,
                    metadata: request.metadata,
                },
                false,
            )
            .await;
        match response {
            Ok(Response::Sent { receipt }) => {
                let result = SendResult {
                    logical_store_id: self.logical_store_id.clone(),
                    operation_id: request.operation_id.clone(),
                    message_id: receipt.id,
                    thread_id: receipt.thread_id,
                    sender: request.sender,
                    recipient: receipt.to,
                    axes: ReceiptAxes {
                        durable_acceptance: EvidenceState::Accepted,
                        occupied_at_acceptance: receipt.occupied,
                        push_acceptance: EvidenceState::Unknown,
                        recipient_consumption: EvidenceState::Unknown,
                        workflow_disposition: EvidenceState::Unknown,
                    },
                    replayed: false,
                };
                let result_json = serde_json::to_string(&result).map_err(invalid)?;
                self.backend
                    .complete_application_operation(
                        &self.logical_store_id.0,
                        &self.responsibility.0,
                        &request.operation_id.0,
                        "accepted",
                        Some(&result_json),
                        None,
                    )
                    .await
                    .map_err(unavailable)?;
                Ok(result)
            }
            Ok(Response::Error {
                code,
                message,
                needs_attach_reason,
            }) => {
                let error = self
                    .registration_error(&request.sender, &code, &message, needs_attach_reason)
                    .await;
                let error_json = serde_json::to_string(&error).map_err(invalid)?;
                let state = if matches!(error, ApplicationClientError::MembershipLost { .. }) {
                    "needs-attach"
                } else {
                    "rejected"
                };
                self.backend
                    .complete_application_operation(
                        &self.logical_store_id.0,
                        &self.responsibility.0,
                        &request.operation_id.0,
                        state,
                        Some(&error_json),
                        None,
                    )
                    .await
                    .map_err(unavailable)?;
                Err(error)
            }
            Ok(other) => Err(ApplicationClientError::Unavailable(format!(
                "unexpected reply response: {other:?}"
            ))),
            Err(error) => {
                let recovery = self.recovery_handle(request.operation_id.clone());
                let _ = self
                    .backend
                    .complete_application_operation(
                        &self.logical_store_id.0,
                        &self.responsibility.0,
                        &request.operation_id.0,
                        "indeterminate",
                        None,
                        Some(&serde_json::to_string(&recovery).map_err(invalid)?),
                    )
                    .await;
                Err(ApplicationClientError::Indeterminate {
                    detail: error.to_string(),
                    recovery,
                })
            }
        }
    }

    pub async fn reconcile_and_send(
        &self,
        request: SendRequest,
        sender_spec: &AddressSpec,
        policy: RecoveryPolicy,
    ) -> Result<SendResult, ApplicationClientError> {
        match self.send(request.clone()).await {
            Err(ApplicationClientError::MembershipLost { .. })
                if matches!(policy, RecoveryPolicy::BoundedRepair { .. }) =>
            {
                self.reconcile(sender_spec, policy).await?;
                self.send(request).await
            }
            result => result,
        }
    }

    pub async fn receive(
        &self,
        address: &str,
        timeout_ms: Option<u64>,
    ) -> Result<Option<ReceivedDelivery>, ApplicationClientError> {
        let membership = self.membership(address)?;
        if membership.handle.capability != StationCapability::Bidirectional {
            return Err(ApplicationClientError::UnsupportedCapability(
                "receive requires bidirectional membership".to_string(),
            ));
        }
        match self
            .request(
                Request::Wait {
                    store_key: self.store_key.clone(),
                    session_id: self.runtime_id.0.clone(),
                    address: address.to_string(),
                    attention: None,
                    min_attention: None,
                    wake_on_cc: false,
                    timeout_ms,
                    waiter_pid: Some(std::process::id()),
                    waiter_start_time: crate::session_watch::capture_process_start_time(
                        std::process::id(),
                    ),
                },
                false,
            )
            .await?
        {
            Response::Message {
                id,
                thread_id,
                parent_id,
                from_addr,
                primary_to,
                cc,
                delivery_role,
                kind,
                attention,
                requires_disposition_for_current_recipient,
                subject,
                body,
                metadata,
                sent_at_ms,
                delivery_id: Some(delivery_id),
                ..
            } => {
                let snapshot_version = self
                    .backend
                    .current_state_version()
                    .await
                    .map_err(unavailable)?;
                let delivery = ExactDeliveryIdentity {
                    logical_store_id: self.logical_store_id.clone(),
                    message_id: id,
                    recipient: address.to_string(),
                    delivery_id,
                };
                Ok(Some(ReceivedDelivery {
                    delivery: delivery.clone(),
                    thread_id,
                    parent_id,
                    from: from_addr,
                    primary_to,
                    cc,
                    delivery_role,
                    kind,
                    attention,
                    requires_disposition: requires_disposition_for_current_recipient,
                    subject,
                    body,
                    metadata,
                    sent_at_ms,
                    snapshot_version,
                    ack: AckHandle {
                        delivery,
                        runtime_id: self.runtime_id.clone(),
                    },
                }))
            }
            Response::Message { .. } => Err(ApplicationClientError::UnsupportedCapability(
                "daemon did not provide exact delivery-row identity".to_string(),
            )),
            Response::Timeout => Ok(None),
            Response::PresenceEnded => Err(ApplicationClientError::MembershipLost {
                address: address.to_string(),
                reason: MembershipLossReason::PredicateDeath,
                detail: "receive presence ended".to_string(),
            }),
            Response::Error {
                code,
                message,
                needs_attach_reason,
            } => Err(self
                .registration_error(address, &code, &message, needs_attach_reason)
                .await),
            other => Err(ApplicationClientError::Unavailable(format!(
                "unexpected receive response: {other:?}"
            ))),
        }
    }

    pub async fn acknowledge(
        &self,
        handle: &AckHandle,
    ) -> Result<AckResult, ApplicationClientError> {
        if handle.runtime_id != self.runtime_id
            || handle.delivery.logical_store_id != self.logical_store_id
        {
            return Err(ApplicationClientError::DeliveryMismatch {
                message_id: handle.delivery.message_id,
                recipient: handle.delivery.recipient.clone(),
                delivery_id: handle.delivery.delivery_id,
            });
        }
        match self
            .request(
                Request::ApplicationAck {
                    store_key: self.store_key.clone(),
                    session_id: self.runtime_id.0.clone(),
                    address: handle.delivery.recipient.clone(),
                    message_id: handle.delivery.message_id,
                    delivery_id: handle.delivery.delivery_id,
                },
                false,
            )
            .await?
        {
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::Marked),
                ..
            } => Ok(AckResult::Marked),
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::AlreadyConsumed),
                ..
            } => Ok(AckResult::AlreadyConsumed),
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::AckNoOp),
                ..
            } => Ok(AckResult::NoDelivery),
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::DeliveryMismatch),
                ..
            } => Err(ApplicationClientError::DeliveryMismatch {
                message_id: handle.delivery.message_id,
                recipient: handle.delivery.recipient.clone(),
                delivery_id: handle.delivery.delivery_id,
            }),
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::NotOwner),
                ..
            } => Err(ApplicationClientError::MembershipLost {
                address: handle.delivery.recipient.clone(),
                reason: MembershipLossReason::OwnerDemoted,
                detail: "ack owner/epoch is stale".to_string(),
            }),
            Response::Error {
                code,
                message,
                needs_attach_reason,
            } => Err(self
                .registration_error(
                    &handle.delivery.recipient,
                    &code,
                    &message,
                    needs_attach_reason,
                )
                .await),
            other => Err(ApplicationClientError::Unavailable(format!(
                "unexpected acknowledgment response: {other:?}"
            ))),
        }
    }

    pub async fn disposition(
        &self,
        sender: &str,
        delivery: &ExactDeliveryIdentity,
        state: &str,
        note: Option<&str>,
    ) -> Result<crate::model::DispositionRow, ApplicationClientError> {
        self.require_sender(sender)?;
        if delivery.logical_store_id != self.logical_store_id {
            return Err(ApplicationClientError::DeliveryMismatch {
                message_id: delivery.message_id,
                recipient: delivery.recipient.clone(),
                delivery_id: delivery.delivery_id,
            });
        }
        let row = self
            .backend
            .delivery_for_recipient(delivery.message_id, &delivery.recipient)
            .await
            .map_err(unavailable)?
            .ok_or_else(|| ApplicationClientError::DeliveryMismatch {
                message_id: delivery.message_id,
                recipient: delivery.recipient.clone(),
                delivery_id: delivery.delivery_id,
            })?;
        if row.id != delivery.delivery_id {
            return Err(ApplicationClientError::DeliveryMismatch {
                message_id: delivery.message_id,
                recipient: delivery.recipient.clone(),
                delivery_id: delivery.delivery_id,
            });
        }
        self.backend
            .insert_disposition(
                delivery.message_id,
                &delivery.recipient,
                state,
                note,
                Some(sender),
            )
            .await
            .map_err(unavailable)
    }

    pub async fn history(
        &self,
        recipient: Option<String>,
        unresolved_only: bool,
        thread_id: Option<i64>,
        since_ms: Option<i64>,
        after_message_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<HistoryItem>, ApplicationClientError> {
        let records = self
            .backend
            .history_page(&HistoryQuery {
                recipient,
                unresolved_only,
                thread_id,
                since_ms,
                after_message_id,
                limit,
                order: HistoryOrder::Ascending,
            })
            .await
            .map_err(unavailable)?;
        Ok(records
            .into_iter()
            .map(|record| HistoryItem {
                logical_store_id: self.logical_store_id.clone(),
                message: record.message,
                delivery: record.delivery,
                latest_disposition: record.latest_disposition,
            })
            .collect())
    }

    pub async fn resolve_source(
        &self,
        source: &SourceReference,
    ) -> Result<SourceResolution, ApplicationClientError> {
        if source.logical_store_id != self.logical_store_id {
            return Ok(SourceResolution::Mismatch);
        }
        Ok(
            match self
                .backend
                .get_message(source.message_id)
                .await
                .map_err(unavailable)?
            {
                Some(message) => SourceResolution::Authoritative(message),
                None => SourceResolution::Unavailable,
            },
        )
    }

    pub async fn reconcile_operation(
        &self,
        operation_id: &OperationId,
    ) -> Result<Option<ApplicationOperationRecord>, ApplicationClientError> {
        self.backend
            .application_operation(
                &self.logical_store_id.0,
                &self.responsibility.0,
                &operation_id.0,
            )
            .await
            .map_err(unavailable)
    }

    pub async fn delta_page(
        &self,
        after_version: i64,
        limit: i64,
    ) -> Result<DeltaPage, ApplicationClientError> {
        let current_version = self
            .backend
            .current_state_version()
            .await
            .map_err(unavailable)?;
        let deltas = self
            .backend
            .state_deltas(after_version, limit)
            .await
            .map_err(unavailable)?;
        if let Some(first) = deltas.first() {
            let expected = after_version.saturating_add(1);
            if first.version != expected {
                return Err(ApplicationClientError::ResyncRequired {
                    expected_version: expected,
                    observed_version: first.version,
                });
            }
        }
        Ok(DeltaPage {
            logical_store_id: self.logical_store_id.clone(),
            from_version: after_version,
            current_version,
            deltas,
        })
    }

    pub async fn declare_compound(
        &self,
        operation_id: &OperationId,
        steps: &[CompoundStep],
    ) -> Result<Vec<CompoundStepRecord>, ApplicationClientError> {
        validate_compound(steps)?;
        let now = now_ms();
        let records: Result<Vec<_>, ApplicationClientError> = steps
            .iter()
            .map(|step| {
                Ok(NewCompoundStepRecord {
                    logical_store_id: self.logical_store_id.0.clone(),
                    application_responsibility: self.responsibility.0.clone(),
                    operation_id: operation_id.0.clone(),
                    step_id: step.step_id.clone(),
                    position: step.position,
                    step_kind: step.kind.clone(),
                    prerequisites_json: serde_json::to_string(&step.prerequisites)
                        .map_err(invalid)?,
                    declaration_json: serde_json::to_string(&step.declaration).map_err(invalid)?,
                    created_at_ms: now,
                })
            })
            .collect();
        self.backend
            .declare_compound_steps(&records?)
            .await
            .map_err(unavailable)
    }

    pub async fn complete_compound_step(
        &self,
        operation_id: &OperationId,
        step_id: &str,
        state: &str,
        outcome: Option<&serde_json::Value>,
        recovery: Option<&serde_json::Value>,
    ) -> Result<CompoundStepRecord, ApplicationClientError> {
        let steps = self
            .backend
            .compound_steps(
                &self.logical_store_id.0,
                &self.responsibility.0,
                &operation_id.0,
            )
            .await
            .map_err(unavailable)?;
        let step = steps
            .iter()
            .find(|step| step.step_id == step_id)
            .ok_or_else(|| {
                ApplicationClientError::InvalidRequest("unknown compound step".into())
            })?;
        let prerequisites: Vec<String> =
            serde_json::from_str(&step.prerequisites_json).map_err(invalid)?;
        for prerequisite in prerequisites {
            let prerequisite = steps
                .iter()
                .find(|candidate| candidate.step_id == prerequisite)
                .ok_or_else(|| {
                    ApplicationClientError::InvalidRequest(
                        "compound prerequisite is not declared".into(),
                    )
                })?;
            if !matches!(
                prerequisite.state.as_str(),
                "accepted" | "completed" | "no-op"
            ) {
                return Err(ApplicationClientError::Partial(format!(
                    "prerequisite {} is {}",
                    prerequisite.step_id, prerequisite.state
                )));
            }
        }
        self.backend
            .complete_compound_step(
                &self.logical_store_id.0,
                &self.responsibility.0,
                &operation_id.0,
                step_id,
                state,
                outcome
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(invalid)?
                    .as_deref(),
                recovery
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(invalid)?
                    .as_deref(),
            )
            .await
            .map_err(unavailable)
    }

    pub async fn cleanup(
        &self,
        policy: RetentionPolicy,
    ) -> Result<CleanupReport, ApplicationClientError> {
        self.backend
            .cleanup_application_records(&self.scope(), policy)
            .await
            .map_err(unavailable)
    }

    pub async fn storage_stats(&self) -> Result<ApplicationStorageStats, ApplicationClientError> {
        self.backend
            .application_storage_stats(&self.scope())
            .await
            .map_err(unavailable)
    }

    pub async fn health(&self) -> Result<Vec<ApplicationHealth>, ApplicationClientError> {
        let status = match self
            .request(
                Request::Status {
                    store_key: Some(self.store_key.clone()),
                    detail: false,
                    proof: None,
                },
                false,
            )
            .await?
        {
            Response::StatusReport { status } => status,
            other => {
                return Err(ApplicationClientError::Unavailable(format!(
                    "unexpected status response: {other:?}"
                )))
            }
        };
        let memberships = self.memberships.lock().unwrap().clone();
        Ok(memberships
            .values()
            .map(|local| {
                let member = status.members.iter().find(|member| {
                    member.session_id == self.runtime_id.0 && member.address == local.handle.address
                });
                health_projection(self, local, member)
            })
            .collect())
    }

    fn require_sender(&self, sender: &str) -> Result<MembershipHandle, ApplicationClientError> {
        let memberships = self.memberships.lock().unwrap();
        if let Some(membership) = memberships.get(sender) {
            return Ok(membership.handle.clone());
        }
        let senders: Vec<_> = memberships.keys().cloned().collect();
        if senders.len() > 1 {
            return Err(ApplicationClientError::AmbiguousSender(senders));
        }
        Err(ApplicationClientError::MembershipLost {
            address: sender.to_string(),
            reason: MembershipLossReason::NeedsAttach,
            detail: "sender is not attached by this application client".to_string(),
        })
    }

    fn membership(&self, address: &str) -> Result<LocalMembership, ApplicationClientError> {
        self.memberships
            .lock()
            .unwrap()
            .get(address)
            .cloned()
            .ok_or_else(|| ApplicationClientError::MembershipLost {
                address: address.to_string(),
                reason: MembershipLossReason::NeedsAttach,
                detail: "address is not attached by this application client".to_string(),
            })
    }

    fn recovery_handle(&self, operation_id: OperationId) -> RecoveryHandle {
        RecoveryHandle {
            logical_store_id: self.logical_store_id.clone(),
            responsibility: self.responsibility.clone(),
            operation_id,
        }
    }

    fn scope(&self) -> ApplicationRecordScope {
        ApplicationRecordScope {
            logical_store_id: self.logical_store_id.0.clone(),
            application_responsibility: self.responsibility.0.clone(),
        }
    }

    async fn request(
        &self,
        request: Request,
        spawn: bool,
    ) -> Result<Response, ApplicationClientError> {
        let response = if spawn {
            crate::daemon::request_connect_or_spawn(&self.store_key, &request).await
        } else {
            let mut client = crate::daemon::connect_existing(&self.store_key)
                .await
                .map_err(unavailable)?;
            client.request(&request).await
        };
        response.map_err(unavailable)
    }
}

fn health_projection(
    client: &ApplicationClient,
    local: &LocalMembership,
    member: Option<&MemberStatus>,
) -> ApplicationHealth {
    let evidence = member
        .and_then(|member| member.health_detail.clone())
        .map(|detail| detail.replace(&client.store_key, &client.logical_store_id.0))
        .into_iter()
        .collect();
    let pending = member
        .map(|member| member.pending_unconsumed_count)
        .unwrap_or_default();
    let actionable = member
        .map(|member| member.inbound_actionable_count)
        .unwrap_or_default();
    let registered = member.is_some();
    let receive_ready = registered
        && local.handle.capability == StationCapability::Bidirectional
        && member
            .map(|member| {
                !member.idle
                    && !matches!(
                        member.station_health,
                        crate::daemon_ipc::StationHealth::UnattendedWithBacklog
                            | crate::daemon_ipc::StationHealth::CoverageConflict
                            | crate::daemon_ipc::StationHealth::Unknown
                    )
            })
            .unwrap_or(false);
    ApplicationHealth {
        logical_store_id: client.logical_store_id.clone(),
        responsibility: client.responsibility.clone(),
        runtime_id: client.runtime_id.clone(),
        address: local.handle.address.clone(),
        capability: local.handle.capability,
        registered,
        lease_epoch: member.map(|member| member.lease_epoch),
        owner_instance_id: member.map(|member| member.owner_instance_id.clone()),
        pending_unconsumed: if local.handle.capability == StationCapability::SendOnly {
            0
        } else {
            pending
        },
        inbound_actionable: if local.handle.capability == StationCapability::SendOnly {
            0
        } else {
            actionable
        },
        acknowledgment_pending: local.handle.capability == StationCapability::Bidirectional
            && pending > 0,
        sender_ready: registered && !member.map(|member| member.idle).unwrap_or(true),
        receive_ready,
        attended_but_deaf: member.map(|member| member.deaf_warn).unwrap_or(false),
        recovering: local.recovering,
        degraded: actionable > 0
            || pending > 0
            || (local.handle.capability == StationCapability::Bidirectional && !receive_ready),
        stopped_or_unattended: member
            .map(|member| {
                member.idle
                    || matches!(
                        member.delivery_mode,
                        DeliveryMode::Unknown | DeliveryMode::Conflict
                    )
            })
            .unwrap_or(true),
        principal: principal_provenance(&client.profile),
        evidence,
    }
}

fn principal_provenance(profile: &BackendProfile) -> PrincipalProvenance {
    match profile.kind.as_str() {
        "sqlite" => PrincipalProvenance {
            principal: Some(crate::config::principal()),
            verification: PrincipalVerification::Unverified,
            evidence: Some("local OS principal; backend does not authenticate it".to_string()),
        },
        _ => PrincipalProvenance {
            principal: None,
            verification: PrincipalVerification::Unavailable,
            evidence: Some(
                "selected backend does not currently expose authenticated principal evidence"
                    .to_string(),
            ),
        },
    }
}

fn payload_fingerprint(request: &SendRequest) -> Result<String, ApplicationClientError> {
    let canonical = serde_json::to_vec(&(
        &request.sender,
        &request.to,
        &request.cc,
        &request.kind,
        &request.attention,
        request.requires_disposition,
        &request.subject,
        &request.body,
        &request.metadata,
    ))
    .map_err(invalid)?;
    let mut hasher = Sha256::new();
    hasher.update(PAYLOAD_FINGERPRINT_DOMAIN);
    hasher.update(canonical);
    Ok(hex(&hasher.finalize()))
}

fn reply_fingerprint(request: &ReplyRequest) -> Result<String, ApplicationClientError> {
    let canonical = serde_json::to_vec(&(
        &request.sender,
        request.message_id,
        &request.cc,
        &request.kind,
        &request.attention,
        request.requires_disposition,
        &request.subject,
        &request.body,
        &request.metadata,
    ))
    .map_err(invalid)?;
    let mut hasher = Sha256::new();
    hasher.update(PAYLOAD_FINGERPRINT_DOMAIN);
    hasher.update(b"reply\0");
    hasher.update(canonical);
    Ok(hex(&hasher.finalize()))
}

fn normalize_cc(values: &[String]) -> Option<String> {
    let mut seen = BTreeSet::new();
    for value in values {
        for part in value
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            seen.insert(part.to_string());
        }
    }
    (!seen.is_empty()).then(|| seen.into_iter().collect::<Vec<_>>().join(","))
}

fn validate_compound(steps: &[CompoundStep]) -> Result<(), ApplicationClientError> {
    let ids: BTreeSet<_> = steps.iter().map(|step| step.step_id.as_str()).collect();
    if ids.len() != steps.len() {
        return Err(ApplicationClientError::InvalidRequest(
            "compound step IDs must be unique".to_string(),
        ));
    }
    for step in steps {
        if step
            .prerequisites
            .iter()
            .any(|prerequisite| !ids.contains(prerequisite.as_str()))
        {
            return Err(ApplicationClientError::InvalidRequest(format!(
                "compound step {} has an undeclared prerequisite",
                step.step_id
            )));
        }
        if step
            .prerequisites
            .iter()
            .any(|value| value == &step.step_id)
        {
            return Err(ApplicationClientError::InvalidRequest(format!(
                "compound step {} depends on itself",
                step.step_id
            )));
        }
    }
    Ok(())
}

fn unavailable(_error: impl fmt::Display) -> ApplicationClientError {
    ApplicationClientError::Unavailable(
        "backend or daemon operation failed; inspect Telex diagnostics for redacted detail"
            .to_string(),
    )
}

fn invalid(error: impl fmt::Display) -> ApplicationClientError {
    ApplicationClientError::InvalidRequest(error.to_string())
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send_request(operation: &str, body: &str) -> SendRequest {
        SendRequest {
            operation_id: OperationId(operation.to_string()),
            sender: "app:sender".to_string(),
            to: "app:recipient".to_string(),
            cc: vec!["observer:b".to_string(), "observer:a".to_string()],
            kind: "note".to_string(),
            attention: "background".to_string(),
            requires_disposition: true,
            subject: Some("subject".to_string()),
            body: body.to_string(),
            metadata: Some("{\"b\":2,\"a\":1}".to_string()),
            retry_budget: 1,
        }
    }

    #[test]
    fn runtime_identity_is_fresh() {
        assert_ne!(RuntimeId::fresh().unwrap(), RuntimeId::fresh().unwrap());
    }

    #[test]
    fn logical_store_identity_is_stable_and_opaque() {
        let id = LogicalStoreId::derive("sqlite:C:\\secret\\telex.db");
        assert_eq!(id, LogicalStoreId::derive("sqlite:C:\\secret\\telex.db"));
        assert!(!id.0.contains("secret"));
        assert!(!id.0.contains("sqlite:"));
    }

    #[test]
    fn payload_fingerprint_is_retry_stable_and_input_sensitive() {
        let first = payload_fingerprint(&send_request("op-1", "one")).unwrap();
        let replay = payload_fingerprint(&send_request("op-1", "one")).unwrap();
        let changed = payload_fingerprint(&send_request("op-1", "two")).unwrap();
        assert_eq!(first, replay);
        assert_ne!(first, changed);
    }

    #[test]
    fn unknown_membership_reason_preserves_raw_evidence() {
        let reason = MembershipLossReason::Unknown {
            raw_reason: Some("future-loss".to_string()),
        };
        let encoded = serde_json::to_string(&reason).unwrap();
        assert!(encoded.contains("future-loss"));
    }

    #[test]
    fn compound_rejects_missing_and_self_prerequisites() {
        let missing = vec![CompoundStep {
            step_id: "close".into(),
            position: 1,
            kind: "disposition".into(),
            prerequisites: vec!["reply".into()],
            declaration: serde_json::json!({}),
        }];
        assert!(validate_compound(&missing).is_err());
        let self_reference = vec![CompoundStep {
            step_id: "reply".into(),
            position: 1,
            kind: "reply".into(),
            prerequisites: vec!["reply".into()],
            declaration: serde_json::json!({}),
        }];
        assert!(validate_compound(&self_reference).is_err());
    }
}
