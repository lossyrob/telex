//! Supported semantic client for long-lived Telex applications.
//!
//! This module is the public application boundary. Daemon frames, backend store
//! keys, paths, connection strings, and backend-specific errors stay private.

use crate::backend::Backend;
use crate::daemon_ipc::{
    DeliveryMode, MemberStatus, NeedsAttachReason, Request, Response,
    StationCapability as WireCapability,
};
use crate::model::{
    now_ms, ApplicationOperationBegin, ApplicationOperationRecord, ApplicationRecordScope,
    ApplicationStorageStats, CleanupReport, CompoundDispositionStep, CompoundStepRecord,
    CompoundStepState, DeliveryOutcome, HistoryOrder, HistoryQuery, NewApplicationOperation,
    NewCompoundStepRecord, RetentionPolicy, StateDeltaRecord, StoreDeltaCleanupReport,
    StoreDeltaRetentionPolicy,
};
use crate::profiles::BackendProfile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex};

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
    fn persisted(value: String) -> Self {
        Self(value)
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
    Protocol {
        code: String,
    },
    RetryableReadiness(String),
    TransportUncertain(String),
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
            Self::Protocol { code } => write!(f, "application protocol error: {code}"),
            Self::RetryableReadiness(detail) => {
                write!(
                    f,
                    "application readiness is temporarily unavailable: {detail}"
                )
            }
            Self::TransportUncertain(detail) => {
                write!(f, "application transport outcome is uncertain: {detail}")
            }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApplicationCapability {
    SendOnly,
    Bidirectional,
}

impl From<ApplicationCapability> for WireCapability {
    fn from(value: ApplicationCapability) -> Self {
        match value {
            ApplicationCapability::SendOnly => WireCapability::SendOnly,
            ApplicationCapability::Bidirectional => WireCapability::Bidirectional,
        }
    }
}

impl From<WireCapability> for ApplicationCapability {
    fn from(value: WireCapability) -> Self {
        match value {
            WireCapability::SendOnly => ApplicationCapability::SendOnly,
            WireCapability::Bidirectional => ApplicationCapability::Bidirectional,
        }
    }
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
    pub capability: ApplicationCapability,
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
    pub capability: ApplicationCapability,
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
    pub validation_error: Option<ApplicationClientError>,
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
    Unavailable,
    NotAttempted,
    Pending,
    Accepted,
    Rejected,
    Disposition(String),
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
    pub capability: ApplicationCapability,
    pub registered: bool,
    pub lease_epoch: Option<i64>,
    pub owner_instance_id: Option<String>,
    pub pending_unconsumed: i64,
    pub inbound_actionable: i64,
    pub acknowledgment_pending: bool,
    pub outstanding_ack_count: usize,
    pub liveness: Vec<ProcessLivenessEvidence>,
    pub sender_ready: bool,
    pub receive_ready: bool,
    pub attended_but_deaf: bool,
    pub recovering: bool,
    pub last_recovery_failure: Option<String>,
    pub degraded: bool,
    pub stopped_or_unattended: bool,
    pub principal: PrincipalProvenance,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessLivenessEvidence {
    pub pid: u32,
    pub start_time: Option<u64>,
    pub alive: bool,
    pub role: String,
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
    pub retained_floor: i64,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompoundDispositionRequest {
    pub sender: String,
    pub delivery: ExactDeliveryIdentity,
    pub state: String,
    pub note: Option<String>,
    pub operation_id: OperationId,
    pub step_id: String,
    pub outcome: Option<serde_json::Value>,
    pub recovery: Option<serde_json::Value>,
}

#[derive(Clone, Debug)]
struct LocalMembership {
    handle: MembershipHandle,
    recovering: bool,
    last_recovery_failure: Option<String>,
}

#[derive(Clone, Debug)]
struct RecoveryAttempt {
    capability: ApplicationCapability,
    recovering: bool,
    last_failure: Option<String>,
}

enum RequestFailure {
    BeforePeerDecision(ApplicationClientError),
    WriteBoundaryUnknown(ApplicationClientError),
}

pub struct ApplicationClient {
    responsibility: ApplicationResponsibility,
    runtime_id: RuntimeId,
    logical_store_id: LogicalStoreId,
    store_key: String,
    profile: BackendProfile,
    backend: Arc<dyn Backend>,
    memberships: Mutex<BTreeMap<String, LocalMembership>>,
    outstanding_acks: Mutex<BTreeSet<(i64, String, i64)>>,
    recovery_attempts: Mutex<BTreeMap<String, RecoveryAttempt>>,
}

pub struct ApplicationStoreMaintenance {
    logical_store_id: LogicalStoreId,
    backend: Arc<dyn Backend>,
}

impl ApplicationStoreMaintenance {
    pub async fn connect(
        backend_name: Option<&str>,
        db_override: Option<&str>,
    ) -> Result<Self, ApplicationClientError> {
        let (_, profile) =
            crate::profiles::resolve(backend_name, db_override).map_err(unavailable)?;
        let backend = crate::profiles::build(&profile, db_override)
            .await
            .map_err(unavailable)?;
        let logical_store_id =
            LogicalStoreId::persisted(backend.logical_store_id().await.map_err(unavailable)?);
        Ok(Self {
            logical_store_id,
            backend,
        })
    }

    pub fn logical_store_id(&self) -> &LogicalStoreId {
        &self.logical_store_id
    }

    pub async fn cleanup_deltas(
        &self,
        policy: StoreDeltaRetentionPolicy,
    ) -> Result<StoreDeltaCleanupReport, ApplicationClientError> {
        self.backend
            .cleanup_state_deltas(policy)
            .await
            .map_err(unavailable)
    }
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
        let logical_store_id =
            LogicalStoreId::persisted(backend.logical_store_id().await.map_err(unavailable)?);
        Ok(Self {
            responsibility: config.responsibility,
            runtime_id: RuntimeId::fresh()?,
            logical_store_id,
            store_key,
            profile,
            backend,
            memberships: Mutex::new(BTreeMap::new()),
            outstanding_acks: Mutex::new(BTreeSet::new()),
            recovery_attempts: Mutex::new(BTreeMap::new()),
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
        let unique: BTreeSet<_> = specs.iter().map(|spec| spec.address.as_str()).collect();
        if unique.len() != specs.len() {
            return MultiAddressOutcome {
                ready: false,
                results: BTreeMap::new(),
                compensation: Vec::new(),
                validation_error: Some(ApplicationClientError::InvalidRequest(
                    "multi-address attach contains duplicate addresses".to_string(),
                )),
            };
        }
        if specs.is_empty() {
            return MultiAddressOutcome {
                ready: true,
                results: BTreeMap::new(),
                compensation: Vec::new(),
                validation_error: None,
            };
        }
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
            validation_error: None,
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
                _ => return Err(unexpected_response("status")),
            };
            if status.members.iter().any(|member| {
                member.session_id == self.runtime_id.0
                    && member.address == spec.address
                    && !member.idle
            }) {
                return Ok(current.handle);
            }
            if self
                .backend
                .detach_tombstone(&self.runtime_id.0, &spec.address)
                .await
                .map_err(unavailable)?
                .is_some()
            {
                return Err(ApplicationClientError::MembershipLost {
                    address: spec.address.clone(),
                    reason: MembershipLossReason::DeliberateDetach,
                    detail: "durable deliberate-detach intent blocks strict recovery".to_string(),
                });
            }
            if let Some(lease) = self
                .backend
                .get_lease(&spec.address)
                .await
                .map_err(unavailable)?
            {
                if lease.owner_instance_id.as_deref()
                    != Some(current.handle.owner_instance_id.as_str())
                {
                    return Err(ApplicationClientError::Collision(CollisionEvidence {
                        address: spec.address.clone(),
                        owner_instance_id: lease.owner_instance_id,
                        lease_epoch: lease.lease_epoch,
                        guidance:
                            "wait for the current owner or use an explicitly authorized reset"
                                .to_string(),
                    }));
                }
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
        self.recovery_attempts.lock().unwrap().insert(
            spec.address.clone(),
            RecoveryAttempt {
                capability: spec.capability,
                recovering: true,
                last_failure: None,
            },
        );
        if let Some(existing) = self.memberships.lock().unwrap().get_mut(&spec.address) {
            existing.recovering = true;
            existing.last_recovery_failure = None;
        }
        loop {
            match self.attach_one(spec, true).await {
                Ok(handle) => {
                    self.recovery_attempts.lock().unwrap().remove(&spec.address);
                    return Ok(handle);
                }
                Err(
                    error @ (ApplicationClientError::MembershipLost { .. }
                    | ApplicationClientError::RetryableReadiness(_)),
                ) if attempt < retries => {
                    attempt += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(50 * attempt as u64)).await;
                    let _ = error;
                }
                Err(error) => {
                    if let Some(existing) = self.memberships.lock().unwrap().get_mut(&spec.address)
                    {
                        existing.recovering = false;
                        existing.last_recovery_failure = Some(error.to_string());
                    }
                    if let Some(recovery) = self
                        .recovery_attempts
                        .lock()
                        .unwrap()
                        .get_mut(&spec.address)
                    {
                        recovery.recovering = false;
                        recovery.last_failure = Some(error.to_string());
                    }
                    return Err(error);
                }
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
            capability: spec.capability.into(),
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
                        last_recovery_failure: None,
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
            _ => Err(unexpected_response("register")),
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
        if code == crate::daemon_ipc::ERROR_COLLISION
            || message.contains("already owned")
            || message.contains("already attended")
        {
            let lease = self.backend.get_lease(address).await.ok().flatten();
            return ApplicationClientError::Collision(CollisionEvidence {
                address: address.to_string(),
                owner_instance_id: lease.as_ref().and_then(|row| row.owner_instance_id.clone()),
                lease_epoch: lease.and_then(|row| row.lease_epoch),
                guidance: "wait for the current owner, reset with explicit authority, or retry within a bounded budget".to_string(),
            });
        }
        if code == crate::daemon_ipc::ERROR_NOT_OWNER {
            return ApplicationClientError::MembershipLost {
                address: address.to_string(),
                reason: MembershipLossReason::OwnerDemoted,
                detail: message.replace(&self.store_key, &self.logical_store_id.0),
            };
        }
        if code == crate::daemon_ipc::ERROR_CAPABILITY_CONFLICT {
            return ApplicationClientError::UnsupportedCapability(
                "membership capability differs from the existing attachment; detach before changing it"
                    .to_string(),
            );
        }
        if code == crate::daemon_ipc::ERROR_NOT_RUNNING && message.contains("draining") {
            return ApplicationClientError::RetryableReadiness("daemon is draining".to_string());
        }
        if code == crate::daemon_ipc::ERROR_UNSUPPORTED {
            return ApplicationClientError::UnsupportedCapability(
                "daemon rejected an unsupported Application Client operation".to_string(),
            );
        }
        if code != crate::daemon_ipc::ERROR_NEEDS_ATTACH {
            return unavailable(format!("{code}: {message}"));
        }
        ApplicationClientError::MembershipLost {
            address: address.to_string(),
            reason: project_membership_loss(reason, code, message),
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
            _ => Err(unexpected_response("detach")),
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
            ApplicationOperationBegin::Replay(existing)
                if matches!(
                    existing.state.as_str(),
                    "accepted" | "completed" | "duplicate"
                ) =>
            {
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
            ApplicationOperationBegin::Replay(existing) if existing.state == "rejected" => {
                return Err(
                    serde_json::from_str(existing.result_json.as_deref().ok_or_else(|| {
                        ApplicationClientError::Protocol {
                            code: "rejected-operation-without-result".to_string(),
                        }
                    })?)
                    .map_err(invalid)?,
                );
            }
            ApplicationOperationBegin::Replay(existing) if existing.state == "pending" => {
                if let Some(reconciled) = self.reconcile_operation(&request.operation_id).await? {
                    if reconciled.state == "accepted" {
                        let mut result: SendResult = serde_json::from_str(
                            reconciled.result_json.as_deref().ok_or_else(|| {
                                ApplicationClientError::Unavailable(
                                    "reconciled operation has no durable result".to_string(),
                                )
                            })?,
                        )
                        .map_err(invalid)?;
                        result.replayed = true;
                        return Ok(result);
                    }
                }
            }
            ApplicationOperationBegin::Replay(existing) if existing.state == "indeterminate" => {
                if let Some(reconciled) = self.reconcile_operation(&request.operation_id).await? {
                    if reconciled.state == "accepted" {
                        let mut result: SendResult = serde_json::from_str(
                            reconciled.result_json.as_deref().ok_or_else(|| {
                                ApplicationClientError::Unavailable(
                                    "reconciled operation has no durable result".to_string(),
                                )
                            })?,
                        )
                        .map_err(invalid)?;
                        result.replayed = true;
                        return Ok(result);
                    }
                }
                return Err(ApplicationClientError::Indeterminate {
                    detail: format!("operation remains {}", existing.state),
                    recovery: self.recovery_handle(request.operation_id),
                });
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

        let daemon_request = Request::ApplicationSend {
            store_key: self.store_key.clone(),
            session_id: self.runtime_id.0.clone(),
            from_addr: request.sender.clone(),
            to_addr: request.to.clone(),
            cc: normalize_cc(&request.cc),
            kind: request.kind.clone(),
            attention: request.attention.clone(),
            requires_disposition: request.requires_disposition,
            subject: request.subject.clone(),
            body: request.body.clone(),
            metadata: request.metadata.clone(),
            logical_store_id: self.logical_store_id.0.clone(),
            application_responsibility: self.responsibility.0.clone(),
            operation_id: request.operation_id.0.clone(),
            payload_fingerprint: operation.payload_fingerprint.clone(),
        };
        let response = match self.request_staged(daemon_request, false).await {
            Ok(response) => response,
            Err(RequestFailure::WriteBoundaryUnknown(error)) => {
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
            Err(RequestFailure::BeforePeerDecision(error)) => return Err(error),
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
                let state = operation_state_for_error(&error);
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
            _ => Err(unexpected_response("send")),
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
            ApplicationOperationBegin::Replay(existing)
                if matches!(
                    existing.state.as_str(),
                    "accepted" | "completed" | "duplicate"
                ) =>
            {
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
            ApplicationOperationBegin::Replay(existing) if existing.state == "rejected" => {
                return Err(
                    serde_json::from_str(existing.result_json.as_deref().ok_or_else(|| {
                        ApplicationClientError::Protocol {
                            code: "rejected-operation-without-result".to_string(),
                        }
                    })?)
                    .map_err(invalid)?,
                );
            }
            ApplicationOperationBegin::Replay(existing) if existing.state == "pending" => {
                if let Some(reconciled) = self.reconcile_operation(&request.operation_id).await? {
                    if reconciled.state == "accepted" {
                        let mut result: SendResult = serde_json::from_str(
                            reconciled.result_json.as_deref().ok_or_else(|| {
                                ApplicationClientError::Unavailable(
                                    "reconciled reply operation has no durable result".to_string(),
                                )
                            })?,
                        )
                        .map_err(invalid)?;
                        result.replayed = true;
                        return Ok(result);
                    }
                }
            }
            ApplicationOperationBegin::Replay(existing) if existing.state == "indeterminate" => {
                if let Some(reconciled) = self.reconcile_operation(&request.operation_id).await? {
                    if reconciled.state == "accepted" {
                        let mut result: SendResult = serde_json::from_str(
                            reconciled.result_json.as_deref().ok_or_else(|| {
                                ApplicationClientError::Unavailable(
                                    "reconciled reply operation has no durable result".to_string(),
                                )
                            })?,
                        )
                        .map_err(invalid)?;
                        result.replayed = true;
                        return Ok(result);
                    }
                }
                return Err(ApplicationClientError::Indeterminate {
                    detail: format!("reply operation remains {}", existing.state),
                    recovery: self.recovery_handle(request.operation_id),
                });
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
            .request_staged(
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
                    logical_store_id: self.logical_store_id.0.clone(),
                    application_responsibility: self.responsibility.0.clone(),
                    operation_id: request.operation_id.0.clone(),
                    payload_fingerprint: operation.payload_fingerprint.clone(),
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
                let state = operation_state_for_error(&error);
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
            Ok(_) => Err(unexpected_response("reply")),
            Err(RequestFailure::WriteBoundaryUnknown(error)) => {
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
            Err(RequestFailure::BeforePeerDecision(error)) => Err(error),
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
        if membership.handle.capability != ApplicationCapability::Bidirectional {
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
                snapshot_version: Some(snapshot_version),
                ..
            } => {
                let delivery = ExactDeliveryIdentity {
                    logical_store_id: self.logical_store_id.clone(),
                    message_id: id,
                    recipient: address.to_string(),
                    delivery_id,
                };
                self.outstanding_acks.lock().unwrap().insert((
                    id,
                    address.to_string(),
                    delivery_id,
                ));
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
            _ => Err(unexpected_response("receive")),
        }
    }

    pub async fn acknowledge(
        &self,
        handle: &AckHandle,
    ) -> Result<AckResult, ApplicationClientError> {
        let membership = self.membership(&handle.delivery.recipient)?;
        if membership.handle.capability != ApplicationCapability::Bidirectional {
            return Err(ApplicationClientError::UnsupportedCapability(
                "acknowledgment requires bidirectional membership".to_string(),
            ));
        }
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
            } => {
                self.clear_outstanding_ack(handle);
                Ok(AckResult::Marked)
            }
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::AlreadyConsumed),
                ..
            } => {
                self.clear_outstanding_ack(handle);
                Ok(AckResult::AlreadyConsumed)
            }
            Response::Ack {
                delivery_outcome: Some(DeliveryOutcome::AckNoOp),
                ..
            } => {
                self.clear_outstanding_ack(handle);
                Ok(AckResult::NoDelivery)
            }
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
            _ => Err(unexpected_response("acknowledge")),
        }
    }

    pub async fn disposition(
        &self,
        sender: &str,
        delivery: &ExactDeliveryIdentity,
        state: &str,
        note: Option<&str>,
    ) -> Result<crate::model::DispositionRow, ApplicationClientError> {
        self.disposition_effect(sender, delivery, state, note, None)
            .await
    }

    pub async fn complete_compound_disposition(
        &self,
        request: &CompoundDispositionRequest,
    ) -> Result<crate::model::DispositionRow, ApplicationClientError> {
        self.disposition_effect(
            &request.sender,
            &request.delivery,
            &request.state,
            request.note.as_deref(),
            Some(CompoundDispositionStep {
                logical_store_id: self.logical_store_id.0.clone(),
                application_responsibility: self.responsibility.0.clone(),
                operation_id: request.operation_id.0.clone(),
                step_id: request.step_id.clone(),
                outcome_json: request
                    .outcome
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(invalid)?,
                recovery_json: request
                    .recovery
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(invalid)?,
            }),
        )
        .await
    }

    async fn disposition_effect(
        &self,
        sender: &str,
        delivery: &ExactDeliveryIdentity,
        state: &str,
        note: Option<&str>,
        compound_step: Option<CompoundDispositionStep>,
    ) -> Result<crate::model::DispositionRow, ApplicationClientError> {
        self.require_sender(sender)?;
        let recipient_membership = self.membership(&delivery.recipient)?;
        if recipient_membership.handle.capability != ApplicationCapability::Bidirectional {
            return Err(ApplicationClientError::UnsupportedCapability(
                "disposition requires bidirectional recipient membership".to_string(),
            ));
        }
        if delivery.logical_store_id != self.logical_store_id {
            return Err(ApplicationClientError::DeliveryMismatch {
                message_id: delivery.message_id,
                recipient: delivery.recipient.clone(),
                delivery_id: delivery.delivery_id,
            });
        }
        let disposition = crate::model::Disposition::parse(state).map_err(invalid)?;
        let terminal = disposition.is_terminal();
        let state = disposition.as_str();
        let (row, outcome) = self
            .backend
            .application_disposition_with_ack(
                &delivery.recipient,
                &recipient_membership.handle.owner_instance_id,
                recipient_membership.handle.lease_epoch,
                delivery.message_id,
                delivery.delivery_id,
                state,
                note,
                Some(sender),
                compound_step.as_ref(),
            )
            .await
            .map_err(unavailable)?;
        match outcome {
            DeliveryOutcome::NotOwner => Err(ApplicationClientError::MembershipLost {
                address: delivery.recipient.clone(),
                reason: MembershipLossReason::OwnerDemoted,
                detail: "disposition owner/epoch is stale".to_string(),
            }),
            DeliveryOutcome::AckNoOp if !terminal => {
                row.ok_or_else(|| ApplicationClientError::Protocol {
                    code: "missing-disposition-result".to_string(),
                })
            }
            DeliveryOutcome::AckNoOp | DeliveryOutcome::DeliveryMismatch => {
                Err(ApplicationClientError::DeliveryMismatch {
                    message_id: delivery.message_id,
                    recipient: delivery.recipient.clone(),
                    delivery_id: delivery.delivery_id,
                })
            }
            DeliveryOutcome::PrerequisiteIncomplete => Err(ApplicationClientError::Partial(
                "compound prerequisite is not durably complete".to_string(),
            )),
            DeliveryOutcome::Marked | DeliveryOutcome::AlreadyConsumed => {
                let row = row.ok_or_else(|| ApplicationClientError::Protocol {
                    code: "missing-disposition-result".to_string(),
                })?;
                if terminal {
                    self.outstanding_acks.lock().unwrap().remove(&(
                        delivery.message_id,
                        delivery.recipient.clone(),
                        delivery.delivery_id,
                    ));
                }
                Ok(row)
            }
        }
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
        let recipient = recipient.ok_or_else(|| {
            ApplicationClientError::InvalidRequest(
                "history requires an exact recipient attached by this client".to_string(),
            )
        })?;
        self.membership(&recipient)?;
        if !(1..=1000).contains(&limit) {
            return Err(ApplicationClientError::InvalidRequest(
                "history limit must be between 1 and 1000".to_string(),
            ));
        }
        let records = self
            .backend
            .history_page(&HistoryQuery {
                recipient: Some(recipient),
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
        self.resolve_source_with_capture(source, None).await
    }

    pub async fn resolve_source_with_capture(
        &self,
        source: &SourceReference,
        captured: Option<crate::model::MessageRow>,
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
                None => captured
                    .map(SourceResolution::CapturedOnly)
                    .unwrap_or(SourceResolution::Unavailable),
            },
        )
    }

    pub async fn reconcile_operation(
        &self,
        operation_id: &OperationId,
    ) -> Result<Option<ApplicationOperationRecord>, ApplicationClientError> {
        let record = self
            .backend
            .application_operation(
                &self.logical_store_id.0,
                &self.responsibility.0,
                &operation_id.0,
            )
            .await
            .map_err(unavailable)?;
        let Some(record) = record else {
            return Ok(None);
        };
        if matches!(record.state.as_str(), "pending" | "indeterminate") {
            if let Some(message) = self
                .backend
                .application_operation_message(
                    &self.logical_store_id.0,
                    &self.responsibility.0,
                    &operation_id.0,
                )
                .await
                .map_err(unavailable)?
            {
                let result = SendResult {
                    logical_store_id: self.logical_store_id.clone(),
                    operation_id: operation_id.clone(),
                    message_id: message.id,
                    thread_id: message.thread_id,
                    sender: message.from_addr.unwrap_or(record.sender.clone()),
                    recipient: message.to_addr,
                    axes: ReceiptAxes {
                        durable_acceptance: EvidenceState::Accepted,
                        occupied_at_acceptance: None,
                        push_acceptance: EvidenceState::Unknown,
                        recipient_consumption: EvidenceState::Unknown,
                        workflow_disposition: EvidenceState::Unknown,
                    },
                    replayed: true,
                };
                return self
                    .backend
                    .complete_application_operation(
                        &self.logical_store_id.0,
                        &self.responsibility.0,
                        &operation_id.0,
                        "accepted",
                        Some(&serde_json::to_string(&result).map_err(invalid)?),
                        None,
                    )
                    .await
                    .map(Some)
                    .map_err(unavailable);
            }
        }
        Ok(Some(record))
    }

    pub async fn refresh_receipt_axes(
        &self,
        operation_id: &OperationId,
    ) -> Result<ReceiptAxes, ApplicationClientError> {
        let record = self
            .reconcile_operation(operation_id)
            .await?
            .ok_or_else(|| {
                ApplicationClientError::InvalidRequest("operation does not exist".to_string())
            })?;
        let result: SendResult =
            serde_json::from_str(record.result_json.as_deref().ok_or_else(|| {
                ApplicationClientError::Indeterminate {
                    detail: format!("operation is {}", record.state),
                    recovery: self.recovery_handle(operation_id.clone()),
                }
            })?)
            .map_err(invalid)?;
        let delivery = self
            .backend
            .delivery_for_recipient(result.message_id, &result.recipient)
            .await
            .map_err(unavailable)?;
        let dispositions = self
            .backend
            .dispositions_for(result.message_id)
            .await
            .map_err(unavailable)?;
        Ok(ReceiptAxes {
            durable_acceptance: EvidenceState::Accepted,
            occupied_at_acceptance: result.axes.occupied_at_acceptance,
            push_acceptance: EvidenceState::Unavailable,
            recipient_consumption: match delivery {
                None => EvidenceState::NotAttempted,
                Some(row) if row.consumed_at_ms.is_some() => EvidenceState::Accepted,
                Some(_) => EvidenceState::Pending,
            },
            workflow_disposition: dispositions
                .iter()
                .rev()
                .find(|row| row.recipient == result.recipient)
                .map(|row| EvidenceState::Disposition(row.state.clone()))
                .unwrap_or(EvidenceState::NotAttempted),
        })
    }

    pub async fn abandon_unmapped_operation(
        &self,
        operation_id: &OperationId,
        reason: &str,
    ) -> Result<ApplicationOperationRecord, ApplicationClientError> {
        let error = ApplicationClientError::InvalidRequest(format!(
            "operation was explicitly abandoned before acceptance: {reason}"
        ));
        self.backend
            .abandon_unmapped_application_operation(
                &self.logical_store_id.0,
                &self.responsibility.0,
                &operation_id.0,
                &serde_json::to_string(&error).map_err(invalid)?,
            )
            .await
            .map_err(unavailable)?
            .ok_or_else(|| {
                ApplicationClientError::InvalidRequest(
                    "operation has acceptance evidence or is no longer abandonable".to_string(),
                )
            })
    }

    pub async fn delta_page(
        &self,
        after_version: i64,
        limit: i64,
    ) -> Result<DeltaPage, ApplicationClientError> {
        if limit <= 0 {
            return Err(ApplicationClientError::InvalidRequest(
                "delta limit must be positive".to_string(),
            ));
        }
        let page = self
            .backend
            .state_delta_page(after_version, limit.min(1000))
            .await
            .map_err(unavailable)?;
        if after_version > page.current_version
            || after_version < page.retained_floor.saturating_sub(1)
        {
            return Err(ApplicationClientError::ResyncRequired {
                expected_version: page.retained_floor.saturating_sub(1),
                observed_version: after_version,
            });
        }
        if let Some(first) = page.deltas.first() {
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
            current_version: page.current_version,
            retained_floor: page.retained_floor,
            deltas: page.deltas,
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
        state: CompoundStepState,
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
            if !CompoundStepState::parse(&prerequisite.state)
                .is_some_and(CompoundStepState::satisfies_prerequisite)
            {
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
                state.as_str(),
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
            _ => return Err(unexpected_response("health-status")),
        };
        let memberships = self.memberships.lock().unwrap().clone();
        let mut health: Vec<_> = memberships
            .values()
            .map(|local| {
                let member = status.members.iter().find(|member| {
                    member.session_id == self.runtime_id.0 && member.address == local.handle.address
                });
                health_projection(self, local, member)
            })
            .collect();
        let recovery_attempts = self.recovery_attempts.lock().unwrap().clone();
        for (address, recovery) in recovery_attempts {
            if memberships.contains_key(&address) {
                continue;
            }
            health.push(ApplicationHealth {
                logical_store_id: self.logical_store_id.clone(),
                responsibility: self.responsibility.clone(),
                runtime_id: self.runtime_id.clone(),
                address,
                capability: recovery.capability,
                registered: false,
                lease_epoch: None,
                owner_instance_id: None,
                pending_unconsumed: 0,
                inbound_actionable: 0,
                acknowledgment_pending: false,
                outstanding_ack_count: 0,
                liveness: Vec::new(),
                sender_ready: false,
                receive_ready: false,
                attended_but_deaf: false,
                recovering: recovery.recovering,
                last_recovery_failure: recovery.last_failure,
                degraded: true,
                stopped_or_unattended: true,
                principal: principal_provenance(&self.profile),
                evidence: vec!["runtime-local recovery attempt; no durable membership".to_string()],
            });
        }
        Ok(health)
    }

    fn require_sender(&self, sender: &str) -> Result<MembershipHandle, ApplicationClientError> {
        let memberships = self.memberships.lock().unwrap();
        if let Some(membership) = memberships.get(sender) {
            return Ok(membership.handle.clone());
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

    fn clear_outstanding_ack(&self, handle: &AckHandle) {
        self.outstanding_acks.lock().unwrap().remove(&(
            handle.delivery.message_id,
            handle.delivery.recipient.clone(),
            handle.delivery.delivery_id,
        ));
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
        match self.request_staged(request, spawn).await {
            Ok(response) => Ok(response),
            Err(RequestFailure::BeforePeerDecision(error)) => Err(error),
            Err(RequestFailure::WriteBoundaryUnknown(error)) => Err(
                ApplicationClientError::TransportUncertain(error.to_string()),
            ),
        }
    }

    async fn request_staged(
        &self,
        request: Request,
        spawn: bool,
    ) -> Result<Response, RequestFailure> {
        let mut client = if spawn {
            crate::daemon::connect_or_spawn(&self.store_key)
                .await
                .map_err(|error| RequestFailure::BeforePeerDecision(unavailable(error)))?
        } else {
            crate::daemon::connect_existing(&self.store_key)
                .await
                .map_err(|error| RequestFailure::BeforePeerDecision(unavailable(error)))?
        };
        if Self::is_application_request(&request)
            && !client
                .ack
                .capabilities
                .iter()
                .any(|capability| capability == crate::daemon_ipc::CAP_APPLICATION_CLIENT_V1)
        {
            return Err(RequestFailure::BeforePeerDecision(
                ApplicationClientError::UnsupportedCapability(
                    "daemon does not advertise application_client_v1".to_string(),
                ),
            ));
        }
        client
            .request(&request)
            .await
            .map_err(|error| RequestFailure::WriteBoundaryUnknown(transport_uncertain(error)))
    }

    fn is_application_request(request: &Request) -> bool {
        matches!(
            request,
            Request::ApplicationRegister { .. }
                | Request::ApplicationAck { .. }
                | Request::ApplicationSend { .. }
                | Request::ApplicationReply { .. }
        )
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
    let outstanding_ack_count = client
        .outstanding_acks
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, recipient, _)| recipient == &local.handle.address)
        .count();
    let receive_ready = registered
        && local.handle.capability == ApplicationCapability::Bidirectional
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
        pending_unconsumed: if local.handle.capability == ApplicationCapability::SendOnly {
            0
        } else {
            pending
        },
        inbound_actionable: if local.handle.capability == ApplicationCapability::SendOnly {
            0
        } else {
            actionable
        },
        acknowledgment_pending: outstanding_ack_count > 0,
        outstanding_ack_count,
        liveness: member
            .map(|member| {
                member
                    .watch_pids
                    .iter()
                    .map(|watch| ProcessLivenessEvidence {
                        pid: watch.pid,
                        start_time: watch.start_time,
                        alive: watch.alive,
                        role: format!("{:?}", watch.role).to_ascii_lowercase(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        sender_ready: registered && !member.map(|member| member.idle).unwrap_or(true),
        receive_ready,
        attended_but_deaf: member.map(|member| member.deaf_warn).unwrap_or(false),
        recovering: local.recovering,
        last_recovery_failure: local.last_recovery_failure.clone(),
        degraded: actionable > 0
            || local.recovering
            || local.last_recovery_failure.is_some()
            || member.map(|member| member.deaf_warn).unwrap_or(false)
            || (local.handle.capability == ApplicationCapability::Bidirectional && !receive_ready),
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
        "postgres" => {
            #[cfg(feature = "postgres")]
            {
                let principal = profile
                    .url
                    .as_deref()
                    .and_then(|url| url.parse::<tokio_postgres::Config>().ok())
                    .and_then(|config| config.get_user().map(str::to_string));
                PrincipalProvenance {
                verification: if principal.is_some() {
                    PrincipalVerification::Unverified
                } else {
                    PrincipalVerification::Unavailable
                },
                principal,
                evidence: Some(
                    "Postgres connection user; authenticated transport evidence is not exposed by the backend"
                        .to_string(),
                ),
                }
            }
            #[cfg(not(feature = "postgres"))]
            {
                PrincipalProvenance {
                    principal: None,
                    verification: PrincipalVerification::Unavailable,
                    evidence: Some("Postgres support is not compiled into this build".to_string()),
                }
            }
        }
        _ => PrincipalProvenance {
            principal: None,
            verification: PrincipalVerification::Unavailable,
            evidence: None,
        },
    }
}

fn operation_state_for_error(error: &ApplicationClientError) -> &'static str {
    match error {
        ApplicationClientError::MembershipLost {
            reason:
                MembershipLossReason::DaemonRestart
                | MembershipLossReason::PredicateDeath
                | MembershipLossReason::NeedsAttach
                | MembershipLossReason::OwnerDemoted,
            ..
        } => "needs-attach",
        _ => "rejected",
    }
}

fn project_membership_loss(
    reason: Option<NeedsAttachReason>,
    code: &str,
    message: &str,
) -> MembershipLossReason {
    match reason {
        Some(NeedsAttachReason::RestartLost) => MembershipLossReason::DaemonRestart,
        Some(NeedsAttachReason::DeliberatelyDetached) => MembershipLossReason::DeliberateDetach,
        Some(NeedsAttachReason::Unknown) => MembershipLossReason::Unknown {
            raw_reason: Some(message.to_string()),
        },
        None if code == crate::daemon_ipc::ERROR_NEEDS_ATTACH => MembershipLossReason::NeedsAttach,
        None => MembershipLossReason::Unknown {
            raw_reason: Some(code.to_string()),
        },
    }
}

fn payload_fingerprint(request: &SendRequest) -> Result<String, ApplicationClientError> {
    let cc = normalize_cc(&request.cc);
    let canonical = serde_json::to_vec(&(
        &request.sender,
        &request.to,
        &cc,
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
    let cc = normalize_cc(&request.cc);
    let canonical = serde_json::to_vec(&(
        &request.sender,
        request.message_id,
        &cc,
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
    let mut indegree: BTreeMap<&str, usize> = steps
        .iter()
        .map(|step| (step.step_id.as_str(), step.prerequisites.len()))
        .collect();
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for step in steps {
        for prerequisite in &step.prerequisites {
            dependents
                .entry(prerequisite.as_str())
                .or_default()
                .push(step.step_id.as_str());
        }
    }
    let mut ready: VecDeque<&str> = indegree
        .iter()
        .filter_map(|(step_id, count)| (*count == 0).then_some(*step_id))
        .collect();
    let mut visited = 0;
    while let Some(step_id) = ready.pop_front() {
        visited += 1;
        for dependent in dependents.get(step_id).into_iter().flatten() {
            let count = indegree.get_mut(dependent).expect("declared compound step");
            *count -= 1;
            if *count == 0 {
                ready.push_back(dependent);
            }
        }
    }
    if visited != steps.len() {
        return Err(ApplicationClientError::InvalidRequest(
            "compound step declaration contains a dependency cycle".to_string(),
        ));
    }
    Ok(())
}

fn unavailable(error: impl fmt::Display) -> ApplicationClientError {
    let detail = error.to_string();
    let lower = detail.to_ascii_lowercase();
    let category = if lower.contains("schema version") {
        "schema-version"
    } else if lower.contains("timeout") || lower.contains("timed out") {
        "timeout"
    } else if lower.contains("connect") || lower.contains("connection") {
        "connection"
    } else if lower.contains("disk") || lower.contains("database is full") {
        "storage"
    } else if lower.contains("unauthorized") || lower.contains("permission") {
        "authorization"
    } else if lower.contains("protocol") || lower.contains("incompatible") {
        "protocol"
    } else {
        "backend"
    };
    let mut hasher = Sha256::new();
    hasher.update(b"telex-application-error-v1\0");
    hasher.update(detail.as_bytes());
    let diagnostic = hex(&hasher.finalize());
    ApplicationClientError::Unavailable(format!(
        "{category} operation failed (diagnostic {})",
        &diagnostic[..16]
    ))
}

fn transport_uncertain(error: impl fmt::Display) -> ApplicationClientError {
    let detail = error.to_string();
    let mut hasher = Sha256::new();
    hasher.update(b"telex-application-transport-v1\0");
    hasher.update(detail.as_bytes());
    let diagnostic = hex(&hasher.finalize());
    ApplicationClientError::TransportUncertain(format!(
        "request may have crossed the daemon boundary (diagnostic {})",
        &diagnostic[..16]
    ))
}

fn invalid(error: impl fmt::Display) -> ApplicationClientError {
    ApplicationClientError::InvalidRequest(error.to_string())
}

fn unexpected_response(code: &'static str) -> ApplicationClientError {
    ApplicationClientError::Protocol {
        code: code.to_string(),
    }
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
    #[cfg(feature = "sqlite")]
    use crate::backend::sqlite::SqliteBackend;

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

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn logical_store_identity_is_persisted_and_opaque() {
        let sqlite_path = std::env::temp_dir().join(format!(
            "telex-store-identity-{}-{}.db",
            std::process::id(),
            now_ms()
        ));
        let path = sqlite_path.to_string_lossy().into_owned();
        let first = SqliteBackend::open(&path).unwrap();
        first.init_schema().await.unwrap();
        let first_id = first.logical_store_id().await.unwrap();
        drop(first);
        let second = SqliteBackend::open(&path).unwrap();
        second.init_schema().await.unwrap();
        let second_id = second.logical_store_id().await.unwrap();
        assert_eq!(first_id, second_id);
        assert!(first_id.starts_with("store-v1-"));
        assert!(!first_id.contains(&path));
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn postgres_principal_hint_is_unverified() {
        let profile = BackendProfile {
            kind: "postgres".into(),
            url: Some("postgres://application-user@example.com/app".into()),
            schema: None,
            path: None,
            auth: None,
            password_env: None,
            password_command: None,
            entra_cred: None,
            entra_scope: None,
        };
        let provenance = principal_provenance(&profile);
        assert_eq!(provenance.principal.as_deref(), Some("application-user"));
        assert_eq!(provenance.verification, PrincipalVerification::Unverified);
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
    fn payload_fingerprint_normalizes_cc_order_and_duplicates() {
        let mut first = send_request("op-1", "one");
        first.cc = vec!["observer:a".into(), "observer:b".into()];
        let mut reordered = first.clone();
        reordered.cc = vec![
            "observer:b".into(),
            "observer:a".into(),
            "observer:a".into(),
        ];
        assert_eq!(
            payload_fingerprint(&first).unwrap(),
            payload_fingerprint(&reordered).unwrap()
        );
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
    fn known_membership_loss_reasons_project_without_collapse() {
        assert_eq!(
            project_membership_loss(
                Some(NeedsAttachReason::RestartLost),
                crate::daemon_ipc::ERROR_NEEDS_ATTACH,
                "restart"
            ),
            MembershipLossReason::DaemonRestart
        );
        assert_eq!(
            project_membership_loss(
                Some(NeedsAttachReason::DeliberatelyDetached),
                crate::daemon_ipc::ERROR_NEEDS_ATTACH,
                "detached"
            ),
            MembershipLossReason::DeliberateDetach
        );
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

        let cycle = vec![
            CompoundStep {
                step_id: "reply".into(),
                position: 1,
                kind: "reply".into(),
                prerequisites: vec!["close".into()],
                declaration: serde_json::json!({}),
            },
            CompoundStep {
                step_id: "close".into(),
                position: 2,
                kind: "disposition".into(),
                prerequisites: vec!["reply".into()],
                declaration: serde_json::json!({}),
            },
        ];
        assert!(validate_compound(&cycle).is_err());
    }

    #[test]
    fn unavailable_errors_are_distinguishable_without_echoing_sources() {
        let timeout = unavailable("timeout connecting to postgres://user:secret@example/db");
        let storage = unavailable("database is full at C:\\secret\\telex.db");
        assert_ne!(timeout.to_string(), storage.to_string());
        assert!(!timeout.to_string().contains("secret"));
        assert!(!storage.to_string().contains("C:\\secret"));
    }

    #[test]
    fn unknown_wire_membership_reason_deserializes() {
        let reason: NeedsAttachReason = serde_json::from_str("\"future_reason\"").unwrap();
        assert_eq!(reason, NeedsAttachReason::Unknown);
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn pending_operation_reconciles_from_atomic_message_mapping() {
        let path = std::env::temp_dir()
            .join(format!(
                "telex-application-reconcile-{}-{}.db",
                std::process::id(),
                now_ms()
            ))
            .to_string_lossy()
            .into_owned();
        let profile = crate::profiles::implicit_sqlite(Some(&path));
        let store_key = crate::profiles::store_key(&profile, Some(&path));
        let backend = Arc::new(SqliteBackend::open(&path).unwrap());
        backend.init_schema().await.unwrap();
        let logical_store_id = LogicalStoreId::persisted(backend.logical_store_id().await.unwrap());
        let client = ApplicationClient {
            responsibility: ApplicationResponsibility("watcher".into()),
            runtime_id: RuntimeId::fresh().unwrap(),
            logical_store_id: logical_store_id.clone(),
            store_key,
            profile,
            backend: backend.clone(),
            memberships: Mutex::new(BTreeMap::new()),
            outstanding_acks: Mutex::new(BTreeSet::new()),
            recovery_attempts: Mutex::new(BTreeMap::new()),
        };
        backend
            .begin_application_operation(&NewApplicationOperation {
                logical_store_id: logical_store_id.0.clone(),
                application_responsibility: "watcher".into(),
                operation_id: "op-crash-window".into(),
                operation_kind: "send".into(),
                sender: "watcher:sender".into(),
                recipients_json: r#"["target"]"#.into(),
                payload_fingerprint: "fingerprint".into(),
                retry_budget: 1,
                created_at_ms: now_ms(),
            })
            .await
            .unwrap();
        backend
            .insert_application_message(
                &crate::model::NewMessage {
                    from_addr: Some("watcher:sender".into()),
                    to_addr: "target".into(),
                    kind: "note".into(),
                    attention: crate::model::Attention::Background,
                    body: "payload".into(),
                    sent_at_ms: now_ms(),
                    ..Default::default()
                },
                &crate::model::ApplicationMessageOperation {
                    logical_store_id: logical_store_id.0,
                    application_responsibility: "watcher".into(),
                    operation_id: "op-crash-window".into(),
                    payload_fingerprint: "fingerprint".into(),
                },
            )
            .await
            .unwrap();

        let reconciled = client
            .reconcile_operation(&OperationId("op-crash-window".into()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reconciled.state, "accepted");
        let result: SendResult =
            serde_json::from_str(reconciled.result_json.as_deref().unwrap()).unwrap();
        assert_eq!(result.recipient, "target");
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn fully_pruned_delta_history_requires_resync() {
        let path = std::env::temp_dir()
            .join(format!(
                "telex-application-pruned-{}-{}.db",
                std::process::id(),
                now_ms()
            ))
            .to_string_lossy()
            .into_owned();
        let profile = crate::profiles::implicit_sqlite(Some(&path));
        let store_key = crate::profiles::store_key(&profile, Some(&path));
        let backend = Arc::new(SqliteBackend::open(&path).unwrap());
        backend.init_schema().await.unwrap();
        let client = ApplicationClient {
            responsibility: ApplicationResponsibility("watcher".into()),
            runtime_id: RuntimeId::fresh().unwrap(),
            logical_store_id: LogicalStoreId::persisted(backend.logical_store_id().await.unwrap()),
            store_key,
            profile,
            backend: backend.clone(),
            memberships: Mutex::new(BTreeMap::new()),
            outstanding_acks: Mutex::new(BTreeSet::new()),
            recovery_attempts: Mutex::new(BTreeMap::new()),
        };
        for index in 0..3 {
            backend
                .append_state_delta("test", &format!("entity:{index}"), "{}")
                .await
                .unwrap();
        }
        backend
            .cleanup_state_deltas(StoreDeltaRetentionPolicy {
                before_version: i64::MAX,
                max_delete: 100,
            })
            .await
            .unwrap();
        assert!(matches!(
            client.delta_page(0, 10).await,
            Err(ApplicationClientError::ResyncRequired { .. })
        ));
        assert!(matches!(
            client.delta_page(100, 10).await,
            Err(ApplicationClientError::ResyncRequired { .. })
        ));
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn attach_rejects_duplicates_and_defines_empty_as_noop() {
        let path = std::env::temp_dir()
            .join(format!(
                "telex-application-attach-validation-{}-{}.db",
                std::process::id(),
                now_ms()
            ))
            .to_string_lossy()
            .into_owned();
        let client = ApplicationClient::connect(ApplicationClientConfig {
            responsibility: ApplicationResponsibility("watcher".into()),
            backend: None,
            db_override: Some(path),
        })
        .await
        .unwrap();
        let empty = client.attach(&[]).await;
        assert!(empty.ready);
        assert!(empty.results.is_empty());
        let duplicate = client
            .attach(&[
                AddressSpec {
                    address: "watcher:sender".into(),
                    capability: ApplicationCapability::SendOnly,
                    description: None,
                    scope: None,
                    tags: None,
                },
                AddressSpec {
                    address: "watcher:sender".into(),
                    capability: ApplicationCapability::SendOnly,
                    description: None,
                    scope: None,
                    tags: None,
                },
            ])
            .await;
        assert!(!duplicate.ready);
        assert!(duplicate.validation_error.is_some());
        assert!(client.memberships.lock().unwrap().is_empty());
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn history_requires_attached_recipient_and_bounded_limit() {
        let path = std::env::temp_dir()
            .join(format!(
                "telex-application-history-{}-{}.db",
                std::process::id(),
                now_ms()
            ))
            .to_string_lossy()
            .into_owned();
        let profile = crate::profiles::implicit_sqlite(Some(&path));
        let store_key = crate::profiles::store_key(&profile, Some(&path));
        let backend = Arc::new(SqliteBackend::open(&path).unwrap());
        backend.init_schema().await.unwrap();
        let logical_store_id = LogicalStoreId::persisted(backend.logical_store_id().await.unwrap());
        let client = ApplicationClient {
            responsibility: ApplicationResponsibility("station".into()),
            runtime_id: RuntimeId::fresh().unwrap(),
            logical_store_id: logical_store_id.clone(),
            store_key,
            profile,
            backend,
            memberships: Mutex::new(BTreeMap::from([(
                "station:inbox".into(),
                LocalMembership {
                    handle: MembershipHandle {
                        logical_store_id,
                        responsibility: ApplicationResponsibility("station".into()),
                        runtime_id: RuntimeId("runtime".into()),
                        address: "station:inbox".into(),
                        capability: ApplicationCapability::Bidirectional,
                        lease_epoch: 1,
                        owner_instance_id: "owner".into(),
                    },
                    recovering: false,
                    last_recovery_failure: None,
                },
            )])),
            outstanding_acks: Mutex::new(BTreeSet::new()),
            recovery_attempts: Mutex::new(BTreeMap::new()),
        };
        assert!(client
            .history(None, false, None, None, None, 10)
            .await
            .is_err());
        assert!(client
            .history(Some("other".into()), false, None, None, None, 10)
            .await
            .is_err());
        assert!(client
            .history(Some("station:inbox".into()), false, None, None, None, 0)
            .await
            .is_err());
        let local = client
            .memberships
            .lock()
            .unwrap()
            .get("station:inbox")
            .unwrap()
            .clone();
        let health = health_projection(&client, &local, None);
        assert_eq!(health.outstanding_ack_count, 0);
        assert!(health.liveness.is_empty());
        assert!(!health.recovering);
        assert!(health.last_recovery_failure.is_none());
    }
}
