//! Supported Rust binding for long-lived Telex applications.
//!
//! This module is the public application boundary. Daemon frames, backend store
//! keys, paths, connection strings, and backend-specific errors stay private.
//! The root `telex` crate version governs Rust source compatibility; serialized
//! Rust values are not a stable wire format or cross-language contract.
//!
//! All async methods run on the caller's Tokio runtime. The binding creates no
//! runtime or sidecar. Callers that may cancel multi-address lifecycle work use
//! [`LifecycleOperation`] so completed work, compensation, an uncertain in-flight
//! address, and untouched addresses remain distinguishable. Receive cancellation
//! never acknowledges a delivery. Retryable send and reply calls require a
//! persisted [`RecoveryHandle`] prepared before the first attempt.

use crate::backend::Backend;
use crate::daemon_ipc::{
    DeliveryMode, MemberStatus, NeedsAttachReason, Request, Response,
    StationCapability as WireCapability,
};
use crate::model::{
    now_ms, ApplicationDetachIntent, ApplicationOperationBegin, ApplicationOperationRecord,
    ApplicationOperationSnapshot, ApplicationRecordScope, ApplicationStorageStats, CleanupReport,
    CompoundDispositionStep, CompoundStepRecord, CompoundStepState, DeliveryOutcome, HistoryOrder,
    HistoryQuery, NewApplicationOperation, NewCompoundStepRecord, RetentionPolicy,
    StateDeltaRecord, StoreDeltaCleanupReport, StoreDeltaRetentionPolicy,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RejectionRetryability {
    Transient,
    Permanent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayloadIdentity {
    pub algorithm: String,
    pub digest: String,
    pub comparable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayloadMismatchEvidence {
    pub attempted: PayloadIdentity,
    pub existing: PayloadIdentity,
}

impl PayloadIdentity {
    fn sha256(digest: String) -> Self {
        Self {
            algorithm: "sha256".to_string(),
            comparable: digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
            digest,
        }
    }
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
        evidence: Box<PayloadMismatchEvidence>,
    },
    StoreBindingMismatch {
        staged: LogicalStoreId,
        current: LogicalStoreId,
    },
    ResyncRequired {
        expected_version: i64,
        observed_version: i64,
    },
    Partial(String),
    Indeterminate {
        detail: String,
        recovery: Box<RecoveryHandle>,
    },
    InvalidRequest(String),
    Protocol {
        code: String,
    },
    RejectedBeforeAcceptance {
        code: String,
        retryability: RejectionRetryability,
        detail: String,
    },
    DeliveryQuarantined {
        message_id: i64,
        recipient: String,
        serialized_bytes: usize,
        max_bytes: usize,
        may_continue: bool,
    },
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
            Self::OperationMismatch { operation_id, .. } => {
                write!(
                    f,
                    "operation {} was reused with different input",
                    operation_id.0
                )
            }
            Self::StoreBindingMismatch { .. } => {
                write!(f, "operation reconciliation store binding does not match")
            }
            Self::ResyncRequired { .. } => write!(f, "state resynchronization required"),
            Self::Partial(detail) => write!(f, "partial application operation: {detail}"),
            Self::Indeterminate { detail, .. } => write!(f, "indeterminate operation: {detail}"),
            Self::InvalidRequest(detail) => write!(f, "invalid application request: {detail}"),
            Self::Protocol { code } => write!(f, "application protocol error: {code}"),
            Self::RejectedBeforeAcceptance {
                code, retryability, ..
            } => write!(
                f,
                "operation rejected before acceptance: {code} ({retryability:?})"
            ),
            Self::DeliveryQuarantined {
                message_id,
                recipient,
                may_continue,
                ..
            } => write!(
                f,
                "delivery {message_id} for {recipient} was quarantined; continue receiving: {may_continue}"
            ),
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
    pub action: CompensationAction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompensationAction {
    Detach,
    Reattach(AddressSpec),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AddressLifecycleResult {
    Attached(MembershipHandle),
    Reconciled(MembershipHandle),
    Detached(MembershipHandle),
    Failed(ApplicationClientError),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiAddressOutcome {
    pub ready: bool,
    pub results: BTreeMap<String, AddressLifecycleResult>,
    pub compensation: Vec<CompensationHandle>,
    pub validation_error: Option<ApplicationClientError>,
    #[serde(default)]
    pub cancellation: Option<LifecycleCancellationEvidence>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// The multi-address lifecycle action described by cancellation evidence.
pub enum LifecycleOperationKind {
    Attach,
    Reconcile,
    Detach,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Typed evidence retained when a multi-address lifecycle operation is canceled.
pub struct LifecycleCancellationEvidence {
    /// The lifecycle action that was canceled.
    pub operation: LifecycleOperationKind,
    /// The address whose request was in flight and may have committed.
    pub may_have_committed: Option<String>,
    /// Addresses for which no request was started.
    pub not_attempted: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryHandle {
    pub logical_store_id: LogicalStoreId,
    pub responsibility: ApplicationResponsibility,
    pub operation_id: OperationId,
    pub payload_identity: PayloadIdentity,
    #[serde(default)]
    pub retention_generation: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotRecordedEvidence {
    pub logical_store_id: LogicalStoreId,
    pub responsibility: ApplicationResponsibility,
    pub operation_id: OperationId,
    pub payload_identity: PayloadIdentity,
    pub retention_generation: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationTarget {
    Send { to: String, cc: Vec<String> },
    Reply { message_id: i64, cc: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordedOperationOutcome {
    Accepted(SendResult),
    Rejected(ApplicationClientError),
    Partial {
        error: Option<ApplicationClientError>,
        recovery: Option<RecoveryHandle>,
    },
    Indeterminate {
        error: Option<ApplicationClientError>,
        recovery: RecoveryHandle,
    },
    Duplicate(SendResult),
    Pending {
        recovery: RecoveryHandle,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedOperation {
    pub logical_store_id: LogicalStoreId,
    pub responsibility: ApplicationResponsibility,
    pub operation_id: OperationId,
    pub sender: String,
    pub target: OperationTarget,
    pub payload_identity: PayloadIdentity,
    pub retry_budget: u32,
    pub outcome: RecordedOperationOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationReconciliation {
    Recorded(Box<RecordedOperation>),
    NotRecorded(NotRecordedEvidence),
    RetentionBoundaryCrossed {
        staged_generation: Option<i64>,
        current_generation: i64,
    },
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
    Quarantined {
        by_principal: String,
        disposition: String,
    },
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
    pub payload_identity: PayloadIdentity,
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
    pub lifecycle: Vec<ApplicationLifecycleEvidence>,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReconciliationEvidence {
    InProgress,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplicationLifecycleEvidence {
    MembershipLoss {
        reason: MembershipLossReason,
        detail: String,
    },
    Collision(CollisionEvidence),
    CompensationPending(CompensationHandle),
    DeliberateDetach {
        runtime_id: RuntimeId,
        reason: String,
        at_ms: i64,
    },
    Reconciliation {
        state: ReconciliationEvidence,
        detail: Option<String>,
    },
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
    spec: AddressSpec,
    recovering: bool,
    last_recovery_failure: Option<String>,
}

#[derive(Clone, Debug)]
struct RecoveryAttempt {
    capability: ApplicationCapability,
    recovering: bool,
    last_failure: Option<String>,
}

#[derive(Clone, Debug)]
struct LifecycleObservation {
    capability: ApplicationCapability,
    evidence: Vec<ApplicationLifecycleEvidence>,
}

enum RequestFailure {
    BeforePeerDecision(ApplicationClientError),
    WriteBoundaryUnknown(ApplicationClientError),
}

enum PeerFailureDisposition {
    Rejected,
    NeedsAttach,
    Indeterminate,
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
    lifecycle_observations: Mutex<BTreeMap<String, LifecycleObservation>>,
}

#[derive(Clone, Copy)]
enum LifecycleAction {
    Attach,
    Reconcile(RecoveryPolicy),
    Detach,
}

/// Caller-owned progress for a cancellation-safe multi-address lifecycle action.
///
/// Drive the operation with [`Self::run`] or one [`Self::advance`] at a time.
/// If the driving future is canceled, call [`Self::cancelled_outcome`] on the
/// retained operation to recover completed results, compensation, and the exact
/// uncertain/not-attempted partition.
#[must_use = "drive the lifecycle operation or obtain its cancelled outcome"]
pub struct LifecycleOperation<'a> {
    client: &'a ApplicationClient,
    action: LifecycleAction,
    specs: Vec<AddressSpec>,
    next_index: usize,
    in_flight: Option<String>,
    results: BTreeMap<String, AddressLifecycleResult>,
    compensation: Vec<CompensationHandle>,
    validation_error: Option<ApplicationClientError>,
    finished: bool,
    #[cfg(test)]
    test_gate: Option<TestGate>,
}

#[cfg(test)]
#[derive(Clone)]
struct TestGate {
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

#[cfg(test)]
fn receive_test_gates() -> &'static Mutex<BTreeMap<String, TestGate>> {
    static GATES: std::sync::OnceLock<Mutex<BTreeMap<String, TestGate>>> =
        std::sync::OnceLock::new();
    GATES.get_or_init(|| Mutex::new(BTreeMap::new()))
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

impl<'a> LifecycleOperation<'a> {
    fn new(
        client: &'a ApplicationClient,
        action: LifecycleAction,
        specs: Vec<AddressSpec>,
    ) -> Self {
        let operation = match action {
            LifecycleAction::Attach => "attach",
            LifecycleAction::Reconcile(_) => "reconcile",
            LifecycleAction::Detach => "detach",
        };
        Self {
            client,
            action,
            validation_error: validate_address_set(&specs, operation),
            specs,
            next_index: 0,
            in_flight: None,
            results: BTreeMap::new(),
            compensation: Vec::new(),
            finished: false,
            #[cfg(test)]
            test_gate: None,
        }
    }

    /// Returns the lifecycle action performed by this operation.
    pub fn kind(&self) -> LifecycleOperationKind {
        match self.action {
            LifecycleAction::Attach => LifecycleOperationKind::Attach,
            LifecycleAction::Reconcile(_) => LifecycleOperationKind::Reconcile,
            LifecycleAction::Detach => LifecycleOperationKind::Detach,
        }
    }

    /// Advances one address and records its result before returning.
    ///
    /// `false` means validation failed or every address is complete.
    pub async fn advance(&mut self) -> bool {
        if self.finished || self.validation_error.is_some() || self.next_index >= self.specs.len() {
            self.finished = true;
            return false;
        }

        let spec = self.specs[self.next_index].clone();
        let previous = self
            .client
            .memberships
            .lock()
            .unwrap()
            .get(&spec.address)
            .cloned();
        self.in_flight = Some(spec.address.clone());
        #[cfg(test)]
        if let Some(gate) = &self.test_gate {
            gate.started.notify_one();
            gate.release.notified().await;
        }

        let result = match self.action {
            LifecycleAction::Attach => self
                .client
                .attach_one(&spec, false)
                .await
                .map(AddressLifecycleResult::Attached),
            LifecycleAction::Reconcile(policy) => self
                .client
                .reconcile(&spec, policy)
                .await
                .map(AddressLifecycleResult::Reconciled),
            LifecycleAction::Detach => self.client.detach(&spec.address).await.map(|()| {
                AddressLifecycleResult::Detached(
                    previous
                        .as_ref()
                        .expect("detach succeeded without a local membership")
                        .handle
                        .clone(),
                )
            }),
        };

        self.record_completed(spec, previous, result);
        true
    }

    fn record_completed(
        &mut self,
        spec: AddressSpec,
        previous: Option<LocalMembership>,
        result: Result<AddressLifecycleResult, ApplicationClientError>,
    ) {
        if result.is_ok() {
            let action = match self.action {
                LifecycleAction::Attach => attach_compensation(previous.as_ref(), &spec),
                LifecycleAction::Reconcile(RecoveryPolicy::BoundedRepair { .. })
                    if previous.is_none() =>
                {
                    Some(CompensationAction::Detach)
                }
                LifecycleAction::Reconcile(RecoveryPolicy::BoundedRepair { .. }) => {
                    attach_compensation(previous.as_ref(), &spec)
                }
                LifecycleAction::Detach => Some(CompensationAction::Reattach(
                    previous
                        .as_ref()
                        .expect("detach succeeded without a local membership")
                        .spec
                        .clone(),
                )),
                LifecycleAction::Reconcile(RecoveryPolicy::Strict) => None,
            };
            if let Some(action) = action {
                self.compensation.push(CompensationHandle {
                    address: spec.address.clone(),
                    runtime_id: self.client.runtime_id.clone(),
                    action,
                });
            }
        }
        self.in_flight = None;
        self.next_index += 1;
        self.results.insert(
            spec.address,
            result.unwrap_or_else(AddressLifecycleResult::Failed),
        );
        if self.next_index >= self.specs.len() {
            self.finished = true;
        }
    }

    /// Runs all remaining addresses and returns the complete outcome.
    pub async fn run(&mut self) -> MultiAddressOutcome {
        while self.advance().await {}
        self.outcome(None)
    }

    /// Stops the operation and returns all evidence retained so far.
    ///
    /// An address whose request future was canceled is reported as
    /// `may_have_committed`; the caller must reconcile it rather than retrying
    /// based on an assumption of absence.
    pub fn cancelled_outcome(mut self) -> MultiAddressOutcome {
        if self.finished {
            return self.outcome(None);
        }
        self.finished = true;
        let not_attempted_start = self.next_index + usize::from(self.in_flight.is_some());
        let may_have_committed = self.in_flight.take();
        if let Some(address) = &may_have_committed {
            if let LifecycleAction::Reconcile(policy) = self.action {
                let spec = &self.specs[self.next_index];
                self.client.record_reconciliation(
                    spec,
                    ReconciliationEvidence::InProgress,
                    Some(
                        "reconcile was canceled without a terminal outcome; reconcile before retrying"
                            .to_string(),
                    ),
                );
                if matches!(policy, RecoveryPolicy::BoundedRepair { .. }) {
                    self.client.recovery_attempts.lock().unwrap().insert(
                        address.clone(),
                        RecoveryAttempt {
                            capability: spec.capability,
                            recovering: true,
                            last_failure: None,
                        },
                    );
                    if let Some(existing) = self.client.memberships.lock().unwrap().get_mut(address)
                    {
                        existing.recovering = true;
                        existing.last_recovery_failure = None;
                    }
                }
            }
        }
        let cancellation = LifecycleCancellationEvidence {
            operation: self.kind(),
            may_have_committed,
            not_attempted: self.specs[not_attempted_start..]
                .iter()
                .map(|spec| spec.address.clone())
                .collect(),
        };
        self.outcome(Some(cancellation))
    }

    fn outcome(&self, cancellation: Option<LifecycleCancellationEvidence>) -> MultiAddressOutcome {
        if let Some(error) = &self.validation_error {
            return MultiAddressOutcome {
                ready: false,
                results: BTreeMap::new(),
                compensation: Vec::new(),
                validation_error: Some(error.clone()),
                cancellation: None,
            };
        }
        self.client.finish_multi_address_outcome(
            self.results.clone(),
            self.compensation.clone(),
            cancellation,
        )
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
            lifecycle_observations: Mutex::new(BTreeMap::new()),
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

    /// Prepares cancellation-safe multi-address attachment.
    pub fn begin_attach(&self, specs: &[AddressSpec]) -> LifecycleOperation<'_> {
        LifecycleOperation::new(self, LifecycleAction::Attach, specs.to_vec())
    }

    pub async fn attach(&self, specs: &[AddressSpec]) -> MultiAddressOutcome {
        self.begin_attach(specs).run().await
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
                self.record_reconciliation(spec, ReconciliationEvidence::Succeeded, None);
                return Ok(current.handle);
            }
            if self
                .backend
                .application_detach_intent(&self.responsibility.0, &spec.address)
                .await
                .map_err(unavailable)?
                .is_some()
            {
                let error = ApplicationClientError::MembershipLost {
                    address: spec.address.clone(),
                    reason: MembershipLossReason::DeliberateDetach,
                    detail: "durable deliberate-detach intent blocks strict recovery".to_string(),
                };
                self.record_lifecycle_failure(spec, &error);
                return Err(error);
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
                    let error = ApplicationClientError::Collision(CollisionEvidence {
                        address: spec.address.clone(),
                        owner_instance_id: lease.owner_instance_id,
                        lease_epoch: lease.lease_epoch,
                        guidance:
                            "wait for the current owner or use an explicitly authorized reset"
                                .to_string(),
                    });
                    self.record_lifecycle_failure(spec, &error);
                    return Err(error);
                }
            }
            let error = ApplicationClientError::MembershipLost {
                address: spec.address.clone(),
                reason: MembershipLossReason::NeedsAttach,
                detail: "strict membership does not repair a lost attachment".to_string(),
            };
            self.record_lifecycle_failure(spec, &error);
            return Err(error);
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
        self.record_reconciliation(spec, ReconciliationEvidence::InProgress, None);
        if let Some(existing) = self.memberships.lock().unwrap().get_mut(&spec.address) {
            existing.recovering = true;
            existing.last_recovery_failure = None;
        }
        loop {
            match self.attach_one(spec, true).await {
                Ok(handle) => {
                    self.recovery_attempts.lock().unwrap().remove(&spec.address);
                    self.record_reconciliation(spec, ReconciliationEvidence::Succeeded, None);
                    return Ok(handle);
                }
                Err(
                    error @ (ApplicationClientError::MembershipLost { .. }
                    | ApplicationClientError::RejectedBeforeAcceptance {
                        retryability: RejectionRetryability::Transient,
                        ..
                    }),
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
                    self.record_lifecycle_failure(spec, &error);
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
            application_responsibility: self.responsibility.0.clone(),
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
                        spec: spec.clone(),
                        recovering: false,
                        last_recovery_failure: None,
                    },
                );
                self.recovery_attempts.lock().unwrap().remove(&spec.address);
                self.lifecycle_observations
                    .lock()
                    .unwrap()
                    .remove(&spec.address);
                Ok(handle)
            }
            Response::Error {
                code,
                message,
                needs_attach_reason,
            } => {
                let error = self
                    .registration_error(&spec.address, &code, &message, needs_attach_reason)
                    .await;
                self.record_lifecycle_failure(spec, &error);
                Err(error)
            }
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
            return preaccept_rejection(code, message, RejectionRetryability::Permanent);
        }
        if code == crate::daemon_ipc::ERROR_NOT_RUNNING && message.contains("draining") {
            return preaccept_rejection(code, message, RejectionRetryability::Transient);
        }
        if code == crate::daemon_ipc::ERROR_UNSUPPORTED {
            return preaccept_rejection(code, message, RejectionRetryability::Permanent);
        }
        if matches!(
            code,
            crate::daemon_ipc::ERROR_INCOMPATIBLE
                | crate::daemon_ipc::ERROR_UNAUTHORIZED
                | crate::daemon_ipc::ERROR_AMBIGUOUS
        ) {
            return preaccept_rejection(code, message, RejectionRetryability::Permanent);
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
        let local = self.membership(address)?;
        match self
            .request(
                Request::ApplicationDetach {
                    store_key: self.store_key.clone(),
                    session_id: self.runtime_id.0.clone(),
                    application_responsibility: self.responsibility.0.clone(),
                    address: address.to_string(),
                    capability: local.handle.capability.into(),
                },
                false,
            )
            .await?
        {
            Response::Ack { .. } => {
                self.memberships.lock().unwrap().remove(address);
                self.recovery_attempts.lock().unwrap().remove(address);
                self.lifecycle_observations.lock().unwrap().insert(
                    address.to_string(),
                    LifecycleObservation {
                        capability: local.handle.capability,
                        evidence: vec![ApplicationLifecycleEvidence::DeliberateDetach {
                            runtime_id: self.runtime_id.clone(),
                            reason: "ApplicationDetach".to_string(),
                            at_ms: now_ms(),
                        }],
                    },
                );
                Ok(())
            }
            Response::Error {
                code,
                message,
                needs_attach_reason,
            } => {
                let error = self
                    .registration_error(address, &code, &message, needs_attach_reason)
                    .await;
                self.record_lifecycle_failure(&local.spec, &error);
                Err(error)
            }
            _ => Err(unexpected_response("detach")),
        }
    }

    pub async fn reconcile_many(
        &self,
        specs: &[AddressSpec],
        policy: RecoveryPolicy,
    ) -> MultiAddressOutcome {
        self.begin_reconcile_many(specs, policy).run().await
    }

    /// Prepares cancellation-safe multi-address reconciliation.
    pub fn begin_reconcile_many(
        &self,
        specs: &[AddressSpec],
        policy: RecoveryPolicy,
    ) -> LifecycleOperation<'_> {
        LifecycleOperation::new(self, LifecycleAction::Reconcile(policy), specs.to_vec())
    }

    pub async fn detach_many(&self, addresses: &[String]) -> MultiAddressOutcome {
        self.begin_detach_many(addresses).run().await
    }

    /// Prepares cancellation-safe multi-address detachment.
    pub fn begin_detach_many(&self, addresses: &[String]) -> LifecycleOperation<'_> {
        let specs: Vec<_> = addresses
            .iter()
            .map(|address| {
                self.memberships
                    .lock()
                    .unwrap()
                    .get(address)
                    .map(|local| local.spec.clone())
                    .unwrap_or_else(|| AddressSpec {
                        address: address.clone(),
                        capability: ApplicationCapability::Bidirectional,
                        description: None,
                        scope: None,
                        tags: None,
                    })
            })
            .collect();
        LifecycleOperation::new(self, LifecycleAction::Detach, specs)
    }

    pub async fn prepare_send(
        &self,
        request: &SendRequest,
    ) -> Result<RecoveryHandle, ApplicationClientError> {
        self.operation_reference(
            request.operation_id.clone(),
            PayloadIdentity::sha256(payload_fingerprint(request)?),
        )
        .await
    }

    pub async fn prepare_reply(
        &self,
        request: &ReplyRequest,
    ) -> Result<RecoveryHandle, ApplicationClientError> {
        self.operation_reference(
            request.operation_id.clone(),
            PayloadIdentity::sha256(reply_fingerprint(request)?),
        )
        .await
    }

    pub async fn send(&self, request: SendRequest) -> Result<SendResult, ApplicationClientError> {
        self.require_sender(&request.sender)?;
        let fingerprint = payload_fingerprint(&request)?;
        let recovery = self.prepare_send(&request).await?;
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
            ApplicationOperationBegin::FingerprintMismatch(existing) => {
                return Err(ApplicationClientError::OperationMismatch {
                    operation_id: request.operation_id,
                    evidence: Box::new(PayloadMismatchEvidence {
                        attempted: PayloadIdentity::sha256(operation.payload_fingerprint.clone()),
                        existing: PayloadIdentity::sha256(existing.payload_fingerprint),
                    }),
                });
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
                if let Some(reconciled) = self
                    .reconcile_operation_current(
                        &request.operation_id,
                        &PayloadIdentity::sha256(operation.payload_fingerprint.clone()),
                    )
                    .await?
                {
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
                if let Some(reconciled) = self
                    .reconcile_operation_current(
                        &request.operation_id,
                        &PayloadIdentity::sha256(operation.payload_fingerprint.clone()),
                    )
                    .await?
                {
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
                    recovery: Box::new(recovery.clone()),
                });
            }
            ApplicationOperationBegin::Replay(existing) if existing.state == "needs-attach" => {}
            ApplicationOperationBegin::Replay(existing) => {
                return Err(ApplicationClientError::Indeterminate {
                    detail: format!("operation remains {}", existing.state),
                    recovery: Box::new(recovery.clone()),
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
                    recovery: Box::new(recovery),
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
                    payload_identity: PayloadIdentity::sha256(
                        operation.payload_fingerprint.clone(),
                    ),
                    replayed: false,
                };
                let result_json = serde_json::to_string(&result).map_err(invalid)?;
                let completion = self
                    .backend
                    .complete_application_operation(
                        &self.logical_store_id.0,
                        &self.responsibility.0,
                        &request.operation_id.0,
                        "accepted",
                        Some(&result_json),
                        None,
                    )
                    .await;
                classify_accepted_completion(completion, &recovery, "send")?;
                Ok(result)
            }
            Response::Error {
                code,
                message,
                needs_attach_reason,
            } => {
                let error = self
                    .registration_error(
                        &request.sender,
                        &code,
                        &message,
                        needs_attach_reason.clone(),
                    )
                    .await;
                Err(self
                    .finish_peer_failure(
                        &request.operation_id,
                        &recovery,
                        &code,
                        needs_attach_reason,
                        error,
                    )
                    .await?)
            }
            _ => Err(unexpected_response("send")),
        }
    }

    pub async fn reply(&self, request: ReplyRequest) -> Result<SendResult, ApplicationClientError> {
        self.require_sender(&request.sender)?;
        let fingerprint = reply_fingerprint(&request)?;
        let recovery = self.prepare_reply(&request).await?;
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
            ApplicationOperationBegin::FingerprintMismatch(existing) => {
                return Err(ApplicationClientError::OperationMismatch {
                    operation_id: request.operation_id,
                    evidence: Box::new(PayloadMismatchEvidence {
                        attempted: PayloadIdentity::sha256(operation.payload_fingerprint.clone()),
                        existing: PayloadIdentity::sha256(existing.payload_fingerprint),
                    }),
                });
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
                if let Some(reconciled) = self
                    .reconcile_operation_current(
                        &request.operation_id,
                        &PayloadIdentity::sha256(operation.payload_fingerprint.clone()),
                    )
                    .await?
                {
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
                if let Some(reconciled) = self
                    .reconcile_operation_current(
                        &request.operation_id,
                        &PayloadIdentity::sha256(operation.payload_fingerprint.clone()),
                    )
                    .await?
                {
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
                    recovery: Box::new(recovery.clone()),
                });
            }
            ApplicationOperationBegin::Replay(existing) if existing.state == "needs-attach" => {}
            ApplicationOperationBegin::Replay(existing) => {
                return Err(ApplicationClientError::Indeterminate {
                    detail: format!("reply operation remains {}", existing.state),
                    recovery: Box::new(recovery.clone()),
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
                    payload_identity: PayloadIdentity::sha256(
                        operation.payload_fingerprint.clone(),
                    ),
                    replayed: false,
                };
                let result_json = serde_json::to_string(&result).map_err(invalid)?;
                let completion = self
                    .backend
                    .complete_application_operation(
                        &self.logical_store_id.0,
                        &self.responsibility.0,
                        &request.operation_id.0,
                        "accepted",
                        Some(&result_json),
                        None,
                    )
                    .await;
                classify_accepted_completion(completion, &recovery, "reply")?;
                Ok(result)
            }
            Ok(Response::Error {
                code,
                message,
                needs_attach_reason,
            }) => {
                let error = self
                    .registration_error(
                        &request.sender,
                        &code,
                        &message,
                        needs_attach_reason.clone(),
                    )
                    .await;
                Err(self
                    .finish_peer_failure(
                        &request.operation_id,
                        &recovery,
                        &code,
                        needs_attach_reason,
                        error,
                    )
                    .await?)
            }
            Ok(_) => Err(unexpected_response("reply")),
            Err(RequestFailure::WriteBoundaryUnknown(error)) => {
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
                    recovery: Box::new(recovery),
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
        #[cfg(test)]
        let test_gate = {
            receive_test_gates()
                .lock()
                .unwrap()
                .get(&self.runtime_id.0)
                .cloned()
        };
        #[cfg(test)]
        if let Some(gate) = test_gate {
            gate.started.notify_one();
            gate.release.notified().await;
        }
        let membership = self.membership(address)?;
        if membership.handle.capability != ApplicationCapability::Bidirectional {
            return Err(ApplicationClientError::UnsupportedCapability(
                "receive requires bidirectional membership".to_string(),
            ));
        }
        let response = self
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
            .await?;
        self.project_receive_response(address, response).await
    }

    async fn project_receive_response(
        &self,
        address: &str,
        response: Response,
    ) -> Result<Option<ReceivedDelivery>, ApplicationClientError> {
        match response {
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
            Response::PresenceEnded => {
                let error = ApplicationClientError::MembershipLost {
                    address: address.to_string(),
                    reason: MembershipLossReason::PredicateDeath,
                    detail: "receive presence ended".to_string(),
                };
                if let Ok(local) = self.membership(address) {
                    self.record_lifecycle_failure(&local.spec, &error);
                }
                Err(error)
            }
            Response::DeliveryQuarantined {
                message_id,
                recipient,
                serialized_bytes,
                max_bytes,
                may_continue,
            } => Err(ApplicationClientError::DeliveryQuarantined {
                message_id,
                recipient,
                serialized_bytes,
                max_bytes,
                may_continue,
            }),
            Response::Error {
                code,
                message,
                needs_attach_reason,
            } => {
                let error = self
                    .registration_error(address, &code, &message, needs_attach_reason)
                    .await;
                if let Ok(local) = self.membership(address) {
                    self.record_lifecycle_failure(&local.spec, &error);
                }
                Err(error)
            }
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
                delivery_outcome: Some(DeliveryOutcome::AckNoOp | DeliveryOutcome::NoDelivery),
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
            } => {
                let error = ApplicationClientError::MembershipLost {
                    address: handle.delivery.recipient.clone(),
                    reason: MembershipLossReason::OwnerDemoted,
                    detail: "ack owner/epoch is stale".to_string(),
                };
                self.record_lifecycle_failure(&membership.spec, &error);
                Err(error)
            }
            Response::Error {
                code,
                message,
                needs_attach_reason,
            } => {
                let error = self
                    .registration_error(
                        &handle.delivery.recipient,
                        &code,
                        &message,
                        needs_attach_reason,
                    )
                    .await;
                self.record_lifecycle_failure(&membership.spec, &error);
                Err(error)
            }
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
                None,
                compound_step.as_ref(),
            )
            .await
            .map_err(unavailable)?;
        match outcome {
            DeliveryOutcome::NotOwner => {
                let error = ApplicationClientError::MembershipLost {
                    address: delivery.recipient.clone(),
                    reason: MembershipLossReason::OwnerDemoted,
                    detail: "disposition owner/epoch is stale".to_string(),
                };
                self.record_lifecycle_failure(&recipient_membership.spec, &error);
                Err(error)
            }
            DeliveryOutcome::AckNoOp if !terminal => {
                row.ok_or_else(|| ApplicationClientError::Protocol {
                    code: "missing-disposition-result".to_string(),
                })
            }
            DeliveryOutcome::AckNoOp => Err(ApplicationClientError::Protocol {
                code: "terminal-disposition-returned-ack-no-op".to_string(),
            }),
            DeliveryOutcome::NoDelivery | DeliveryOutcome::DeliveryMismatch => {
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
        let membership = self.membership(&recipient)?;
        if membership.handle.capability != ApplicationCapability::Bidirectional {
            return Err(ApplicationClientError::UnsupportedCapability(
                "history requires bidirectional membership".to_string(),
            ));
        }
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

    async fn operation_reference(
        &self,
        operation_id: OperationId,
        payload_identity: PayloadIdentity,
    ) -> Result<RecoveryHandle, ApplicationClientError> {
        if operation_id.0.trim().is_empty() {
            return Err(ApplicationClientError::InvalidRequest(
                "operation id must not be empty".to_string(),
            ));
        }
        if !payload_identity.comparable {
            return Err(ApplicationClientError::InvalidRequest(
                "operation reference requires comparable payload identity".to_string(),
            ));
        }
        let retention_generation = self
            .backend
            .application_operation_retention_generation(&self.scope())
            .await
            .map_err(unavailable)?;
        Ok(RecoveryHandle {
            logical_store_id: self.logical_store_id.clone(),
            responsibility: self.responsibility.clone(),
            operation_id,
            payload_identity,
            retention_generation: Some(retention_generation),
        })
    }

    pub async fn reconcile_operation(
        &self,
        reference: &RecoveryHandle,
    ) -> Result<OperationReconciliation, ApplicationClientError> {
        if reference.logical_store_id != self.logical_store_id
            || reference.responsibility != self.responsibility
        {
            return Err(ApplicationClientError::StoreBindingMismatch {
                staged: reference.logical_store_id.clone(),
                current: self.logical_store_id.clone(),
            });
        }
        let snapshot = self
            .backend
            .application_operation_snapshot(&self.scope(), &reference.operation_id.0)
            .await
            .map_err(unavailable)?;
        if let Some(record) = self
            .reconcile_operation_snapshot(
                snapshot.clone(),
                &reference.operation_id,
                &reference.payload_identity,
            )
            .await?
        {
            return Ok(OperationReconciliation::Recorded(Box::new(
                project_operation_record(record, reference)?,
            )));
        }
        let current_generation = snapshot.retention_generation;
        if reference.retention_generation == Some(current_generation) {
            return Ok(OperationReconciliation::NotRecorded(NotRecordedEvidence {
                logical_store_id: reference.logical_store_id.clone(),
                responsibility: reference.responsibility.clone(),
                operation_id: reference.operation_id.clone(),
                payload_identity: reference.payload_identity.clone(),
                retention_generation: current_generation,
            }));
        }
        Ok(OperationReconciliation::RetentionBoundaryCrossed {
            staged_generation: reference.retention_generation,
            current_generation,
        })
    }

    async fn reconcile_operation_current(
        &self,
        operation_id: &OperationId,
        staged_payload: &PayloadIdentity,
    ) -> Result<Option<ApplicationOperationRecord>, ApplicationClientError> {
        let snapshot = self
            .backend
            .application_operation_snapshot(&self.scope(), &operation_id.0)
            .await
            .map_err(unavailable)?;
        self.reconcile_operation_snapshot(snapshot, operation_id, staged_payload)
            .await
    }

    async fn reconcile_operation_snapshot(
        &self,
        snapshot: ApplicationOperationSnapshot,
        operation_id: &OperationId,
        staged_payload: &PayloadIdentity,
    ) -> Result<Option<ApplicationOperationRecord>, ApplicationClientError> {
        let retention_generation = snapshot.retention_generation;
        let Some(record) = snapshot.operation else {
            return Ok(None);
        };
        let stored_payload = PayloadIdentity::sha256(record.payload_fingerprint.clone());
        if !staged_payload.comparable
            || !stored_payload.comparable
            || staged_payload.algorithm != stored_payload.algorithm
            || staged_payload.digest != stored_payload.digest
        {
            return Err(ApplicationClientError::OperationMismatch {
                operation_id: operation_id.clone(),
                evidence: Box::new(PayloadMismatchEvidence {
                    attempted: staged_payload.clone(),
                    existing: stored_payload,
                }),
            });
        }
        if matches!(record.state.as_str(), "pending" | "indeterminate") {
            if let Some(message) = snapshot.message {
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
                    payload_identity: PayloadIdentity::sha256(record.payload_fingerprint.clone()),
                    replayed: true,
                };
                let recovery = RecoveryHandle {
                    logical_store_id: self.logical_store_id.clone(),
                    responsibility: self.responsibility.clone(),
                    operation_id: operation_id.clone(),
                    payload_identity: staged_payload.clone(),
                    retention_generation: Some(retention_generation),
                };
                return classify_accepted_completion(
                    self.backend
                        .complete_application_operation(
                            &self.logical_store_id.0,
                            &self.responsibility.0,
                            &operation_id.0,
                            "accepted",
                            Some(&serde_json::to_string(&result).map_err(invalid)?),
                            None,
                        )
                        .await,
                    &recovery,
                    "operation reconciliation",
                )
                .map(Some);
            }
        }
        Ok(Some(record))
    }

    pub async fn refresh_receipt_axes(
        &self,
        reference: &RecoveryHandle,
    ) -> Result<ReceiptAxes, ApplicationClientError> {
        let record = match self.reconcile_operation(reference).await? {
            OperationReconciliation::Recorded(record) => record,
            OperationReconciliation::NotRecorded(_) => {
                return Err(ApplicationClientError::InvalidRequest(
                    "operation is authoritatively not recorded".to_string(),
                ))
            }
            OperationReconciliation::RetentionBoundaryCrossed { .. } => {
                return Err(ApplicationClientError::Indeterminate {
                    detail: "operation evidence crossed a retention boundary".to_string(),
                    recovery: Box::new(reference.clone()),
                })
            }
        };
        let result = match record.outcome {
            RecordedOperationOutcome::Accepted(result)
            | RecordedOperationOutcome::Duplicate(result) => result,
            RecordedOperationOutcome::Rejected(error) => return Err(error),
            RecordedOperationOutcome::Pending { recovery }
            | RecordedOperationOutcome::Indeterminate { recovery, .. } => {
                return Err(ApplicationClientError::Indeterminate {
                    detail: "operation remains indeterminate".to_string(),
                    recovery: Box::new(recovery),
                })
            }
            RecordedOperationOutcome::Partial { recovery, .. } => {
                return Err(ApplicationClientError::Indeterminate {
                    detail: "operation is partially complete".to_string(),
                    recovery: Box::new(recovery.unwrap_or_else(|| reference.clone())),
                })
            }
        };
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
        let latest_disposition = dispositions
            .iter()
            .rev()
            .find(|row| row.recipient == result.recipient);
        let quarantine = dispositions.iter().find(|row| {
            row.recipient == result.recipient && row.origin.as_deref() == Some("daemon-quarantine")
        });
        Ok(ReceiptAxes {
            durable_acceptance: EvidenceState::Accepted,
            occupied_at_acceptance: result.axes.occupied_at_acceptance,
            push_acceptance: EvidenceState::Unavailable,
            recipient_consumption: if let Some(row) = quarantine {
                EvidenceState::Quarantined {
                    by_principal: row.by_principal.clone().unwrap_or_default(),
                    disposition: row.state.clone(),
                }
            } else {
                match delivery {
                    None => EvidenceState::NotAttempted,
                    Some(row) if row.consumed_at_ms.is_some() => EvidenceState::Accepted,
                    Some(_) => EvidenceState::Pending,
                }
            },
            workflow_disposition: latest_disposition
                .map(|row| {
                    if row.origin.as_deref() == Some("daemon-quarantine") {
                        EvidenceState::Quarantined {
                            by_principal: row.by_principal.clone().unwrap_or_default(),
                            disposition: row.state.clone(),
                        }
                    } else {
                        EvidenceState::Disposition(row.state.clone())
                    }
                })
                .unwrap_or(EvidenceState::NotAttempted),
        })
    }

    pub async fn abandon_unmapped_operation(
        &self,
        reference: &RecoveryHandle,
        reason: &str,
    ) -> Result<ApplicationOperationRecord, ApplicationClientError> {
        if reference.logical_store_id != self.logical_store_id
            || reference.responsibility != self.responsibility
        {
            return Err(ApplicationClientError::StoreBindingMismatch {
                staged: reference.logical_store_id.clone(),
                current: self.logical_store_id.clone(),
            });
        }
        let error = ApplicationClientError::InvalidRequest(format!(
            "operation was explicitly abandoned before acceptance: {reason}"
        ));
        self.backend
            .abandon_unmapped_application_operation(
                &self.logical_store_id.0,
                &self.responsibility.0,
                &reference.operation_id.0,
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

    async fn finish_peer_failure(
        &self,
        operation_id: &OperationId,
        recovery: &RecoveryHandle,
        code: &str,
        needs_attach_reason: Option<NeedsAttachReason>,
        error: ApplicationClientError,
    ) -> Result<ApplicationClientError, ApplicationClientError> {
        let error_json = serde_json::to_string(&error).map_err(invalid)?;
        match classify_peer_failure(code, needs_attach_reason) {
            PeerFailureDisposition::Rejected => {
                self.backend
                    .complete_application_operation(
                        &self.logical_store_id.0,
                        &self.responsibility.0,
                        &operation_id.0,
                        "rejected",
                        Some(&error_json),
                        None,
                    )
                    .await
                    .map_err(unavailable)?;
                Ok(error)
            }
            PeerFailureDisposition::NeedsAttach => {
                self.backend
                    .complete_application_operation(
                        &self.logical_store_id.0,
                        &self.responsibility.0,
                        &operation_id.0,
                        "needs-attach",
                        Some(&error_json),
                        None,
                    )
                    .await
                    .map_err(unavailable)?;
                Ok(error)
            }
            PeerFailureDisposition::Indeterminate => {
                self.backend
                    .complete_application_operation(
                        &self.logical_store_id.0,
                        &self.responsibility.0,
                        &operation_id.0,
                        "indeterminate",
                        Some(&error_json),
                        Some(&serde_json::to_string(recovery).map_err(invalid)?),
                    )
                    .await
                    .map_err(unavailable)?;
                Ok(ApplicationClientError::Indeterminate {
                    detail: error.to_string(),
                    recovery: Box::new(recovery.clone()),
                })
            }
        }
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
            .await
        {
            Ok(Response::StatusReport { status }) => Some(status),
            Ok(_) => return Err(unexpected_response("health-status")),
            Err(_) => None,
        };
        let memberships = self.memberships.lock().unwrap().clone();
        let lifecycle = self.lifecycle_observations.lock().unwrap().clone();
        let mut health: BTreeMap<_, _> = memberships
            .values()
            .map(|local| {
                let member = status.as_ref().and_then(|status| {
                    status.members.iter().find(|member| {
                        member.session_id == self.runtime_id.0
                            && member.address == local.handle.address
                    })
                });
                let mut projected = health_projection(self, local, member);
                let membership_loss = status.as_ref().and_then(|status| {
                    status.membership_losses.iter().find(|loss| {
                        loss.session_id == self.runtime_id.0
                            && loss.address == local.handle.address
                            && loss.store_key == self.store_key
                    })
                });
                if let Some(loss) = membership_loss {
                    projected.registered = false;
                    projected.sender_ready = false;
                    projected.receive_ready = false;
                    projected.stopped_or_unattended = true;
                    projected
                        .lifecycle
                        .push(ApplicationLifecycleEvidence::MembershipLoss {
                            reason: project_membership_loss(
                                Some(loss.reason.clone()),
                                crate::daemon_ipc::ERROR_NEEDS_ATTACH,
                                &loss.detail,
                            ),
                            detail: loss.detail.clone(),
                        });
                } else if let Some(member) = member {
                    if member.owner_instance_id != local.handle.owner_instance_id
                        || member.lease_epoch != local.handle.lease_epoch
                    {
                        projected
                            .lifecycle
                            .push(ApplicationLifecycleEvidence::MembershipLoss {
                                reason: MembershipLossReason::OwnerDemoted,
                                detail: "health observed a different durable owner or lease epoch"
                                    .to_string(),
                            });
                    }
                } else if let Some(other) = status.as_ref().and_then(|status| {
                    status
                        .members
                        .iter()
                        .find(|member| member.address == local.handle.address && !member.idle)
                }) {
                    projected
                        .lifecycle
                        .push(ApplicationLifecycleEvidence::Collision(CollisionEvidence {
                            address: local.handle.address.clone(),
                            owner_instance_id: Some(other.owner_instance_id.clone()),
                            lease_epoch: Some(other.lease_epoch),
                            guidance:
                                "wait for the current owner or use an explicitly authorized reset"
                                    .to_string(),
                        }));
                } else {
                    projected
                        .lifecycle
                        .push(ApplicationLifecycleEvidence::MembershipLoss {
                            reason: if status.is_some() {
                                MembershipLossReason::DaemonRestart
                            } else {
                                MembershipLossReason::Unknown {
                                    raw_reason: Some("daemon status unavailable".to_string()),
                                }
                            },
                            detail: "runtime-local membership is absent from daemon status"
                                .to_string(),
                        });
                }
                if let Some(observation) = lifecycle.get(&local.handle.address) {
                    projected.lifecycle.extend(observation.evidence.clone());
                }
                projected.degraded |= !projected.lifecycle.is_empty();
                (local.handle.address.clone(), projected)
            })
            .collect();
        let recovery_attempts = self.recovery_attempts.lock().unwrap().clone();
        for (address, recovery) in recovery_attempts {
            if health.contains_key(&address) {
                continue;
            }
            health.insert(
                address.clone(),
                ApplicationHealth {
                    logical_store_id: self.logical_store_id.clone(),
                    responsibility: self.responsibility.clone(),
                    runtime_id: self.runtime_id.clone(),
                    address: address.clone(),
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
                    last_recovery_failure: recovery.last_failure.clone(),
                    degraded: true,
                    stopped_or_unattended: true,
                    principal: principal_provenance(&self.profile),
                    lifecycle: lifecycle
                        .get(&address)
                        .map(|observation| observation.evidence.clone())
                        .unwrap_or_else(|| {
                            vec![ApplicationLifecycleEvidence::Reconciliation {
                                state: if recovery.recovering {
                                    ReconciliationEvidence::InProgress
                                } else {
                                    ReconciliationEvidence::Failed
                                },
                                detail: recovery.last_failure.clone(),
                            }]
                        }),
                    evidence: vec![
                        "runtime-local recovery attempt; no durable membership".to_string()
                    ],
                },
            );
        }
        for intent in self
            .backend
            .application_detach_intents(&self.responsibility.0)
            .await
            .map_err(unavailable)?
        {
            let mut projected = detached_health(self, intent);
            if let Some(observation) = lifecycle.get(&projected.address) {
                projected.lifecycle.extend(observation.evidence.clone());
            }
            health.insert(projected.address.clone(), projected);
        }
        for (address, observation) in lifecycle {
            health
                .entry(address.clone())
                .or_insert_with(|| ApplicationHealth {
                    logical_store_id: self.logical_store_id.clone(),
                    responsibility: self.responsibility.clone(),
                    runtime_id: self.runtime_id.clone(),
                    address,
                    capability: observation.capability,
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
                    recovering: false,
                    last_recovery_failure: None,
                    degraded: true,
                    stopped_or_unattended: true,
                    principal: principal_provenance(&self.profile),
                    lifecycle: observation.evidence,
                    evidence: vec!["runtime-local lifecycle evidence".to_string()],
                });
        }
        Ok(health.into_values().collect())
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

    fn clear_outstanding_ack(&self, handle: &AckHandle) {
        self.outstanding_acks.lock().unwrap().remove(&(
            handle.delivery.message_id,
            handle.delivery.recipient.clone(),
            handle.delivery.delivery_id,
        ));
    }

    fn record_reconciliation(
        &self,
        spec: &AddressSpec,
        state: ReconciliationEvidence,
        detail: Option<String>,
    ) {
        self.lifecycle_observations.lock().unwrap().insert(
            spec.address.clone(),
            LifecycleObservation {
                capability: spec.capability,
                evidence: vec![ApplicationLifecycleEvidence::Reconciliation { state, detail }],
            },
        );
    }

    fn record_lifecycle_failure(&self, spec: &AddressSpec, error: &ApplicationClientError) {
        let evidence = match error {
            ApplicationClientError::MembershipLost { reason, detail, .. } => {
                ApplicationLifecycleEvidence::MembershipLoss {
                    reason: reason.clone(),
                    detail: detail.clone(),
                }
            }
            ApplicationClientError::Collision(collision) => {
                ApplicationLifecycleEvidence::Collision(collision.clone())
            }
            _ => ApplicationLifecycleEvidence::Reconciliation {
                state: ReconciliationEvidence::Failed,
                detail: Some(error.to_string()),
            },
        };
        self.lifecycle_observations.lock().unwrap().insert(
            spec.address.clone(),
            LifecycleObservation {
                capability: spec.capability,
                evidence: vec![evidence],
            },
        );
    }

    fn finish_multi_address_outcome(
        &self,
        results: BTreeMap<String, AddressLifecycleResult>,
        mut compensation: Vec<CompensationHandle>,
        cancellation: Option<LifecycleCancellationEvidence>,
    ) -> MultiAddressOutcome {
        let ready = cancellation.is_none()
            && results
                .values()
                .all(|value| !matches!(value, AddressLifecycleResult::Failed(_)));
        if ready {
            compensation.clear();
        } else {
            let mut observations = self.lifecycle_observations.lock().unwrap();
            for handle in &compensation {
                let capability = match &handle.action {
                    CompensationAction::Reattach(spec) => spec.capability,
                    CompensationAction::Detach => self
                        .memberships
                        .lock()
                        .unwrap()
                        .get(&handle.address)
                        .map(|local| local.handle.capability)
                        .unwrap_or(ApplicationCapability::Bidirectional),
                };
                observations
                    .entry(handle.address.clone())
                    .or_insert_with(|| LifecycleObservation {
                        capability,
                        evidence: Vec::new(),
                    })
                    .evidence
                    .push(ApplicationLifecycleEvidence::CompensationPending(
                        handle.clone(),
                    ));
            }
        }
        MultiAddressOutcome {
            ready,
            results,
            compensation,
            validation_error: None,
            cancellation,
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
                | Request::ApplicationDetach { .. }
        )
    }
}

fn health_projection(
    client: &ApplicationClient,
    local: &LocalMembership,
    member: Option<&MemberStatus>,
) -> ApplicationHealth {
    let bidirectional = local.handle.capability == ApplicationCapability::Bidirectional;
    let evidence = if bidirectional {
        member
            .and_then(|member| member.health_detail.clone())
            .map(|detail| detail.replace(&client.store_key, &client.logical_store_id.0))
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };
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
        attended_but_deaf: bidirectional && member.map(|member| member.deaf_warn).unwrap_or(false),
        recovering: local.recovering,
        last_recovery_failure: local.last_recovery_failure.clone(),
        degraded: (bidirectional
            && (actionable > 0 || member.map(|member| member.deaf_warn).unwrap_or(false)))
            || local.recovering
            || local.last_recovery_failure.is_some()
            || (bidirectional && !receive_ready),
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
        lifecycle: Vec::new(),
        evidence,
    }
}

fn detached_health(
    client: &ApplicationClient,
    intent: ApplicationDetachIntent,
) -> ApplicationHealth {
    let capability = match intent.capability.as_str() {
        "send-only" => ApplicationCapability::SendOnly,
        _ => ApplicationCapability::Bidirectional,
    };
    ApplicationHealth {
        logical_store_id: client.logical_store_id.clone(),
        responsibility: client.responsibility.clone(),
        runtime_id: client.runtime_id.clone(),
        address: intent.address,
        capability,
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
        recovering: false,
        last_recovery_failure: None,
        degraded: true,
        stopped_or_unattended: true,
        principal: principal_provenance(&client.profile),
        lifecycle: vec![
            ApplicationLifecycleEvidence::MembershipLoss {
                reason: MembershipLossReason::DeliberateDetach,
                detail: "durable deliberate-detach intent blocks automatic repair".to_string(),
            },
            ApplicationLifecycleEvidence::DeliberateDetach {
                runtime_id: RuntimeId(intent.runtime_id),
                reason: intent.reason,
                at_ms: intent.at_ms,
            },
        ],
        evidence: vec!["durable application detach intent".to_string()],
    }
}

fn validate_address_set(specs: &[AddressSpec], operation: &str) -> Option<ApplicationClientError> {
    let unique: BTreeSet<_> = specs.iter().map(|spec| spec.address.as_str()).collect();
    (unique.len() != specs.len()).then(|| {
        ApplicationClientError::InvalidRequest(format!(
            "multi-address {operation} contains duplicate addresses"
        ))
    })
}

fn attach_compensation(
    previous: Option<&LocalMembership>,
    requested: &AddressSpec,
) -> Option<CompensationAction> {
    match previous {
        None => Some(CompensationAction::Detach),
        Some(previous) if previous.spec != *requested => {
            Some(CompensationAction::Reattach(previous.spec.clone()))
        }
        Some(_) => None,
    }
}

fn classify_accepted_completion<T>(
    completion: anyhow::Result<T>,
    recovery: &RecoveryHandle,
    operation_kind: &str,
) -> Result<T, ApplicationClientError> {
    completion.map_err(|error| ApplicationClientError::Indeterminate {
        detail: format!(
            "{operation_kind} was durably accepted but local result persistence failed: {}",
            redacted_diagnostic("storage", &error.to_string())
        ),
        recovery: Box::new(recovery.clone()),
    })
}

fn project_operation_record(
    record: ApplicationOperationRecord,
    reference: &RecoveryHandle,
) -> Result<RecordedOperation, ApplicationClientError> {
    let target = match record.operation_kind.as_str() {
        "send" => {
            let (to, cc) = serde_json::from_str::<(String, Vec<String>)>(&record.recipients_json)
                .or_else(|_| {
                    serde_json::from_str::<Vec<String>>(&record.recipients_json).and_then(
                        |mut recipients| {
                            if recipients.is_empty() {
                                return Err(serde::de::Error::custom(
                                    "send operation has no recipient",
                                ));
                            }
                            Ok((recipients.remove(0), recipients))
                        },
                    )
                })
                .map_err(invalid)?;
            OperationTarget::Send { to, cc }
        }
        "reply" => {
            let (message_id, cc): (i64, Vec<String>) =
                serde_json::from_str(&record.recipients_json).map_err(invalid)?;
            OperationTarget::Reply { message_id, cc }
        }
        _ => {
            return Err(ApplicationClientError::Protocol {
                code: "unknown-operation-kind".to_string(),
            })
        }
    };
    let stored_recovery = record
        .recovery_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(invalid)?;
    let result = || -> Result<SendResult, ApplicationClientError> {
        serde_json::from_str(record.result_json.as_deref().ok_or_else(|| {
            ApplicationClientError::Protocol {
                code: "terminal-operation-without-result".to_string(),
            }
        })?)
        .map_err(invalid)
    };
    let error = || -> Result<ApplicationClientError, ApplicationClientError> {
        serde_json::from_str(record.result_json.as_deref().ok_or_else(|| {
            ApplicationClientError::Protocol {
                code: "terminal-operation-without-result".to_string(),
            }
        })?)
        .map_err(invalid)
    };
    let outcome = match record.state.as_str() {
        "accepted" | "completed" => RecordedOperationOutcome::Accepted(result()?),
        "duplicate" => RecordedOperationOutcome::Duplicate(result()?),
        "rejected" | "needs-attach" => RecordedOperationOutcome::Rejected(error()?),
        "partial" => RecordedOperationOutcome::Partial {
            error: record
                .result_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(invalid)?,
            recovery: stored_recovery,
        },
        "indeterminate" => RecordedOperationOutcome::Indeterminate {
            error: record
                .result_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(invalid)?,
            recovery: stored_recovery.unwrap_or_else(|| reference.clone()),
        },
        "pending" => RecordedOperationOutcome::Pending {
            recovery: stored_recovery.unwrap_or_else(|| reference.clone()),
        },
        _ => {
            return Err(ApplicationClientError::Protocol {
                code: "unknown-operation-state".to_string(),
            })
        }
    };
    Ok(RecordedOperation {
        logical_store_id: LogicalStoreId::persisted(record.logical_store_id),
        responsibility: ApplicationResponsibility(record.application_responsibility),
        operation_id: OperationId(record.operation_id),
        sender: record.sender,
        target,
        payload_identity: PayloadIdentity::sha256(record.payload_fingerprint),
        retry_budget: u32::try_from(record.retry_budget).map_err(invalid)?,
        outcome,
    })
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

fn classify_peer_failure(
    code: &str,
    needs_attach_reason: Option<NeedsAttachReason>,
) -> PeerFailureDisposition {
    if matches!(
        needs_attach_reason.as_ref(),
        Some(NeedsAttachReason::DeliberatelyDetached)
    ) {
        return PeerFailureDisposition::Rejected;
    }
    if code == crate::daemon_ipc::ERROR_NEEDS_ATTACH
        || code == crate::daemon_ipc::ERROR_NOT_OWNER
        || matches!(
            needs_attach_reason.as_ref(),
            Some(NeedsAttachReason::RestartLost | NeedsAttachReason::PredicateDeath)
        )
    {
        return PeerFailureDisposition::NeedsAttach;
    }
    if matches!(
        code,
        crate::daemon_ipc::ERROR_INCOMPATIBLE
            | crate::daemon_ipc::ERROR_UNAUTHORIZED
            | crate::daemon_ipc::ERROR_NOT_RUNNING
            | crate::daemon_ipc::ERROR_AMBIGUOUS
            | crate::daemon_ipc::ERROR_UNSUPPORTED
            | crate::daemon_ipc::ERROR_COLLISION
            | crate::daemon_ipc::ERROR_CAPABILITY_CONFLICT
    ) {
        return PeerFailureDisposition::Rejected;
    }
    PeerFailureDisposition::Indeterminate
}

fn project_membership_loss(
    reason: Option<NeedsAttachReason>,
    code: &str,
    _message: &str,
) -> MembershipLossReason {
    match reason {
        Some(NeedsAttachReason::RestartLost) => MembershipLossReason::DaemonRestart,
        Some(NeedsAttachReason::DeliberatelyDetached) => MembershipLossReason::DeliberateDetach,
        Some(NeedsAttachReason::PredicateDeath) => MembershipLossReason::PredicateDeath,
        Some(NeedsAttachReason::PushIntentPending | NeedsAttachReason::PushIntentUnrecoverable) => {
            MembershipLossReason::NeedsAttach
        }
        Some(NeedsAttachReason::Unknown(raw_reason)) => MembershipLossReason::Unknown {
            raw_reason: Some(raw_reason),
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

fn preaccept_rejection(
    code: &str,
    detail: &str,
    retryability: RejectionRetryability,
) -> ApplicationClientError {
    ApplicationClientError::RejectedBeforeAcceptance {
        code: code.to_string(),
        retryability,
        detail: redacted_diagnostic("pre-acceptance-refusal", detail),
    }
}

fn redacted_diagnostic(category: &str, detail: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"telex-application-public-diagnostic-v1\0");
    hasher.update(category.as_bytes());
    hasher.update(detail.as_bytes());
    let digest = hex(&hasher.finalize());
    format!("{category} (diagnostic {})", &digest[..16])
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

    fn reply_request(operation: &str, body: &str) -> ReplyRequest {
        ReplyRequest {
            operation_id: OperationId(operation.to_string()),
            sender: "app:sender".to_string(),
            message_id: 42,
            cc: vec!["observer:b".to_string(), "observer:a".to_string()],
            kind: "reply".to_string(),
            attention: "background".to_string(),
            requires_disposition: false,
            subject: Some("reply subject".to_string()),
            body: body.to_string(),
            metadata: Some("{\"reply\":true}".to_string()),
            retry_budget: 2,
        }
    }

    #[cfg(feature = "sqlite")]
    async fn sqlite_client(
        name: &str,
        responsibility: &str,
    ) -> (ApplicationClient, Arc<SqliteBackend>) {
        let path = std::env::temp_dir()
            .join(format!(
                "telex-application-{name}-{}-{}.db",
                std::process::id(),
                now_ms()
            ))
            .to_string_lossy()
            .into_owned();
        let profile = crate::profiles::implicit_sqlite(Some(&path));
        let backend = Arc::new(SqliteBackend::open(&path).unwrap());
        backend.init_schema().await.unwrap();
        let client = ApplicationClient {
            responsibility: ApplicationResponsibility(responsibility.to_string()),
            runtime_id: RuntimeId::fresh().unwrap(),
            logical_store_id: LogicalStoreId::persisted(backend.logical_store_id().await.unwrap()),
            store_key: crate::profiles::store_key(&profile, Some(&path)),
            profile,
            backend: backend.clone(),
            memberships: Mutex::new(BTreeMap::new()),
            outstanding_acks: Mutex::new(BTreeSet::new()),
            recovery_attempts: Mutex::new(BTreeMap::new()),
            lifecycle_observations: Mutex::new(BTreeMap::new()),
        };
        (client, backend)
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

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn receive_quarantine_is_typed_and_sender_axes_preserve_provenance() {
        use crate::daemon::test_support::{registered_epoch, TestDaemon};

        let daemon = TestDaemon::new("application-client-quarantine");
        let store_key = daemon.store_key("application-client-quarantine");
        let path = store_key
            .strip_prefix("sqlite:")
            .expect("SQLite test store")
            .to_string();
        let backend = daemon.backend(&store_key).await.unwrap();
        let logical_store_id = LogicalStoreId::persisted(backend.logical_store_id().await.unwrap());
        let runtime_id = RuntimeId("receiver-session".to_string());
        let (lease_epoch, owner_instance_id) =
            registered_epoch(&daemon, &store_key, &runtime_id.0, "receiver").await;
        let client = ApplicationClient {
            responsibility: ApplicationResponsibility("receiver-app".into()),
            runtime_id: runtime_id.clone(),
            logical_store_id: logical_store_id.clone(),
            store_key: store_key.clone(),
            profile: crate::profiles::implicit_sqlite(Some(&path)),
            backend: backend.clone(),
            memberships: Mutex::new(BTreeMap::from([(
                "receiver".to_string(),
                LocalMembership {
                    handle: MembershipHandle {
                        logical_store_id: logical_store_id.clone(),
                        responsibility: ApplicationResponsibility("receiver-app".into()),
                        runtime_id,
                        address: "receiver".into(),
                        capability: ApplicationCapability::Bidirectional,
                        lease_epoch,
                        owner_instance_id,
                    },
                    spec: AddressSpec {
                        address: "receiver".into(),
                        capability: ApplicationCapability::Bidirectional,
                        description: None,
                        scope: None,
                        tags: None,
                    },
                    recovering: false,
                    last_recovery_failure: None,
                },
            )])),
            outstanding_acks: Mutex::new(BTreeSet::new()),
            recovery_attempts: Mutex::new(BTreeMap::new()),
            lifecycle_observations: Mutex::new(BTreeMap::new()),
        };
        let fingerprint = "d".repeat(64);
        let operation = NewApplicationOperation {
            logical_store_id: logical_store_id.0.clone(),
            application_responsibility: "sender-app".into(),
            operation_id: "oversized-send".into(),
            operation_kind: "send".into(),
            sender: "sender".into(),
            recipients_json: r#"["receiver"]"#.into(),
            payload_fingerprint: fingerprint.clone(),
            retry_budget: 1,
            created_at_ms: now_ms(),
        };
        backend
            .begin_application_operation(&operation)
            .await
            .unwrap();
        let oversized = backend
            .insert_application_message(
                &crate::model::NewMessage {
                    from_addr: Some("sender".into()),
                    to_addr: "receiver".into(),
                    kind: "note".into(),
                    attention: crate::model::Attention::Background,
                    body: "x".repeat(crate::daemon_ipc::MAX_JSONL_FRAME_BYTES + 1),
                    sent_at_ms: now_ms(),
                    ..Default::default()
                },
                &crate::model::ApplicationMessageOperation {
                    logical_store_id: logical_store_id.0.clone(),
                    application_responsibility: "sender-app".into(),
                    operation_id: "oversized-send".into(),
                    payload_fingerprint: fingerprint.clone(),
                },
            )
            .await
            .unwrap();
        let accepted = SendResult {
            logical_store_id: logical_store_id.clone(),
            operation_id: OperationId("oversized-send".into()),
            message_id: oversized.id,
            thread_id: oversized.thread_id,
            sender: "sender".into(),
            recipient: "receiver".into(),
            axes: ReceiptAxes {
                durable_acceptance: EvidenceState::Accepted,
                occupied_at_acceptance: Some(true),
                push_acceptance: EvidenceState::Unknown,
                recipient_consumption: EvidenceState::Unknown,
                workflow_disposition: EvidenceState::Unknown,
            },
            payload_identity: PayloadIdentity::sha256(fingerprint.clone()),
            replayed: false,
        };
        backend
            .complete_application_operation(
                &logical_store_id.0,
                "sender-app",
                "oversized-send",
                "accepted",
                Some(&serde_json::to_string(&accepted).unwrap()),
                None,
            )
            .await
            .unwrap();
        let following = backend
            .insert_message(&crate::model::NewMessage {
                from_addr: Some("sender".into()),
                to_addr: "receiver".into(),
                kind: "note".into(),
                attention: crate::model::Attention::Background,
                body: "following".into(),
                sent_at_ms: now_ms(),
                ..Default::default()
            })
            .await
            .unwrap();

        let receive_gate = TestGate {
            started: Arc::new(tokio::sync::Notify::new()),
            release: Arc::new(tokio::sync::Notify::new()),
        };
        receive_test_gates()
            .lock()
            .unwrap()
            .insert(client.runtime_id.0.clone(), receive_gate.clone());
        {
            let cancelled_receive = client.receive("receiver", None);
            tokio::pin!(cancelled_receive);
            tokio::select! {
                _ = receive_gate.started.notified() => {}
                result = &mut cancelled_receive => {
                    panic!("receive completed before cancellation gate: {result:?}");
                }
            }
        }
        receive_test_gates()
            .lock()
            .unwrap()
            .remove(&client.runtime_id.0);
        assert!(backend
            .delivery_for_recipient(following.id, "receiver")
            .await
            .unwrap()
            .unwrap()
            .consumed_at_ms
            .is_none());

        let quarantined = daemon
            .wait(&store_key, "receiver-session", "receiver", 1_000)
            .await;
        assert!(matches!(
            client
                .project_receive_response("receiver", quarantined)
                .await,
            Err(ApplicationClientError::DeliveryQuarantined {
                message_id,
                ref recipient,
                serialized_bytes,
                max_bytes,
                may_continue: true,
            }) if message_id == oversized.id
                && recipient == "receiver"
                && serialized_bytes > max_bytes
        ));
        let received = client
            .project_receive_response(
                "receiver",
                daemon
                    .wait(&store_key, "receiver-session", "receiver", 1_000)
                    .await,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received.delivery.message_id, following.id);
        assert!(backend
            .delivery_for_recipient(following.id, "receiver")
            .await
            .unwrap()
            .unwrap()
            .consumed_at_ms
            .is_none());
        drop(received);
        assert!(backend
            .delivery_for_recipient(following.id, "receiver")
            .await
            .unwrap()
            .unwrap()
            .consumed_at_ms
            .is_none());

        let reopened_backend = Arc::new(SqliteBackend::open(&path).unwrap());
        reopened_backend.init_schema().await.unwrap();
        let sender_client = ApplicationClient {
            responsibility: ApplicationResponsibility("sender-app".into()),
            runtime_id: RuntimeId("sender-session".into()),
            logical_store_id: logical_store_id.clone(),
            store_key: store_key.clone(),
            profile: crate::profiles::implicit_sqlite(Some(&path)),
            backend: reopened_backend.clone(),
            memberships: Mutex::new(BTreeMap::new()),
            outstanding_acks: Mutex::new(BTreeSet::new()),
            recovery_attempts: Mutex::new(BTreeMap::new()),
            lifecycle_observations: Mutex::new(BTreeMap::new()),
        };
        let axes = sender_client
            .refresh_receipt_axes(&RecoveryHandle {
                logical_store_id: logical_store_id.clone(),
                responsibility: ApplicationResponsibility("sender-app".into()),
                operation_id: OperationId("oversized-send".into()),
                payload_identity: PayloadIdentity::sha256(fingerprint),
                retention_generation: None,
            })
            .await
            .unwrap();
        let quarantine = EvidenceState::Quarantined {
            by_principal: "daemon".into(),
            disposition: "rejected".into(),
        };
        assert_eq!(axes.recipient_consumption, quarantine);
        assert_eq!(axes.workflow_disposition, quarantine);
        let before_forgery = reopened_backend
            .state_delta_page(0, 100)
            .await
            .unwrap()
            .current_version;
        reopened_backend
            .insert_disposition(
                oversized.id,
                "receiver",
                "rejected",
                Some("daemon rejected delivery frame: forged"),
                Some("daemon"),
            )
            .await
            .unwrap();
        reopened_backend
            .insert_disposition(
                oversized.id,
                "receiver",
                "handled",
                Some("application completed follow-up"),
                Some("receiver-app"),
            )
            .await
            .unwrap();
        let refreshed = sender_client
            .refresh_receipt_axes(&RecoveryHandle {
                logical_store_id: logical_store_id.clone(),
                responsibility: ApplicationResponsibility("sender-app".into()),
                operation_id: OperationId("oversized-send".into()),
                payload_identity: PayloadIdentity::sha256("d".repeat(64)),
                retention_generation: None,
            })
            .await
            .unwrap();
        assert_eq!(refreshed.recipient_consumption, quarantine);
        assert_eq!(
            refreshed.workflow_disposition,
            EvidenceState::Disposition("handled".into())
        );
        assert_eq!(
            reopened_backend
                .dispositions_for(oversized.id)
                .await
                .unwrap()
                .iter()
                .filter(|row| row.origin.as_deref() == Some("daemon-quarantine"))
                .count(),
            1
        );
        let forgery_deltas = reopened_backend
            .state_delta_page(before_forgery, 100)
            .await
            .unwrap();
        for axis in ["acknowledgment", "disposition"] {
            assert!(forgery_deltas
                .deltas
                .iter()
                .filter(|delta| delta.axis == axis)
                .all(|delta| !delta.payload_json.contains("daemon-quarantine")));
        }
        let deltas = reopened_backend.state_delta_page(0, 100).await.unwrap();
        for kind in ["acknowledgment", "disposition"] {
            assert!(deltas.deltas.iter().any(|delta| {
                delta.axis == kind
                    && delta
                        .payload_json
                        .contains("\"evidence\":\"daemon-quarantine\"")
                    && delta.payload_json.contains("\"by_principal\":\"daemon\"")
            }));
        }
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

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn prepared_recovery_handles_round_trip_and_reconcile() {
        let (client, _) = sqlite_client("prepare-handles", "application").await;
        let send = send_request("prepared-send", "send body");
        let reply = reply_request("prepared-reply", "reply body");

        let send_handle = client.prepare_send(&send).await.unwrap();
        let reply_handle = client.prepare_reply(&reply).await.unwrap();

        assert_eq!(send_handle.operation_id, send.operation_id);
        assert_eq!(
            send_handle.payload_identity,
            PayloadIdentity::sha256(payload_fingerprint(&send).unwrap())
        );
        assert_eq!(reply_handle.operation_id, reply.operation_id);
        assert_eq!(
            reply_handle.payload_identity,
            PayloadIdentity::sha256(reply_fingerprint(&reply).unwrap())
        );
        assert_eq!(send_handle.retention_generation, Some(0));
        assert_eq!(reply_handle.retention_generation, Some(0));
        assert_ne!(
            send_handle.payload_identity.digest,
            reply_handle.payload_identity.digest
        );

        for handle in [&send_handle, &reply_handle] {
            let encoded = serde_json::to_string(handle).unwrap();
            let restored: RecoveryHandle = serde_json::from_str(&encoded).unwrap();
            assert_eq!(&restored, handle);
            match client.reconcile_operation(&restored).await.unwrap() {
                OperationReconciliation::NotRecorded(evidence) => {
                    assert_eq!(evidence.operation_id, handle.operation_id);
                    assert_eq!(evidence.payload_identity, handle.payload_identity);
                }
                other => panic!("expected exact-operation not-recorded evidence, got {other:?}"),
            }
        }
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn accepted_send_and_reply_completion_faults_are_indeterminate() {
        let (client, _) = sqlite_client("accepted-completion-fault", "application").await;
        let send_recovery = client
            .prepare_send(&send_request("send-fault", "body"))
            .await
            .unwrap();
        let reply_recovery = client
            .prepare_reply(&reply_request("reply-fault", "body"))
            .await
            .unwrap();

        for (kind, recovery) in [("send", send_recovery), ("reply", reply_recovery)] {
            let fault: anyhow::Result<ApplicationOperationRecord> =
                Err(anyhow::anyhow!("injected completion failure"));
            match classify_accepted_completion(fault, &recovery, kind) {
                Err(ApplicationClientError::Indeterminate {
                    detail,
                    recovery: actual,
                }) => {
                    assert!(detail.contains("durably accepted"));
                    assert_eq!(*actual, recovery);
                }
                other => panic!("expected {kind} indeterminate result, got {other:?}"),
            }
        }
    }

    #[test]
    fn persisted_operation_state_projects_to_typed_public_outcomes() {
        let result = SendResult {
            logical_store_id: LogicalStoreId::persisted("store-v1-test".into()),
            operation_id: OperationId("operation".into()),
            message_id: 7,
            thread_id: 7,
            sender: "app:sender".into(),
            recipient: "app:recipient".into(),
            axes: ReceiptAxes {
                durable_acceptance: EvidenceState::Accepted,
                occupied_at_acceptance: Some(true),
                push_acceptance: EvidenceState::Unknown,
                recipient_consumption: EvidenceState::Unknown,
                workflow_disposition: EvidenceState::Unknown,
            },
            payload_identity: PayloadIdentity::sha256("a".repeat(64)),
            replayed: false,
        };
        let recovery = RecoveryHandle {
            logical_store_id: result.logical_store_id.clone(),
            responsibility: ApplicationResponsibility("application".into()),
            operation_id: result.operation_id.clone(),
            payload_identity: result.payload_identity.clone(),
            retention_generation: Some(0),
        };
        let base = ApplicationOperationRecord {
            logical_store_id: result.logical_store_id.0.clone(),
            application_responsibility: recovery.responsibility.0.clone(),
            operation_id: result.operation_id.0.clone(),
            operation_kind: "send".into(),
            sender: result.sender.clone(),
            recipients_json: serde_json::to_string(&(
                result.recipient.clone(),
                Vec::<String>::new(),
            ))
            .unwrap(),
            payload_fingerprint: result.payload_identity.digest.clone(),
            retry_budget: 1,
            state: "accepted".into(),
            result_json: Some(serde_json::to_string(&result).unwrap()),
            recovery_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
            completed_at_ms: Some(1),
        };

        assert!(matches!(
            project_operation_record(base.clone(), &recovery)
                .unwrap()
                .outcome,
            RecordedOperationOutcome::Accepted(_)
        ));
        assert!(matches!(
            project_operation_record(
                ApplicationOperationRecord {
                    state: "duplicate".into(),
                    ..base.clone()
                },
                &recovery
            )
            .unwrap()
            .outcome,
            RecordedOperationOutcome::Duplicate(_)
        ));
        let rejected = ApplicationClientError::RejectedBeforeAcceptance {
            code: "Rejected".into(),
            retryability: RejectionRetryability::Permanent,
            detail: "rejected".into(),
        };
        assert!(matches!(
            project_operation_record(
                ApplicationOperationRecord {
                    state: "rejected".into(),
                    result_json: Some(serde_json::to_string(&rejected).unwrap()),
                    ..base.clone()
                },
                &recovery
            )
            .unwrap()
            .outcome,
            RecordedOperationOutcome::Rejected(_)
        ));
        for state in ["partial", "indeterminate", "pending"] {
            let projected = project_operation_record(
                ApplicationOperationRecord {
                    state: state.into(),
                    result_json: (state != "pending")
                        .then(|| serde_json::to_string(&rejected).unwrap()),
                    recovery_json: Some(serde_json::to_string(&recovery).unwrap()),
                    completed_at_ms: None,
                    ..base.clone()
                },
                &recovery,
            )
            .unwrap();
            assert!(matches!(
                (state, projected.outcome),
                ("partial", RecordedOperationOutcome::Partial { .. })
                    | (
                        "indeterminate",
                        RecordedOperationOutcome::Indeterminate { .. }
                    )
                    | ("pending", RecordedOperationOutcome::Pending { .. })
            ));
        }
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn health_projects_typed_loss_collision_compensation_and_detach_evidence() {
        let (client, backend) = sqlite_client("typed-health", "application").await;
        let reasons = [
            MembershipLossReason::DaemonRestart,
            MembershipLossReason::PredicateDeath,
            MembershipLossReason::OwnerDemoted,
            MembershipLossReason::Unknown {
                raw_reason: Some("future-loss".into()),
            },
        ];
        for (index, reason) in reasons.into_iter().enumerate() {
            client.lifecycle_observations.lock().unwrap().insert(
                format!("lost:{index}"),
                LifecycleObservation {
                    capability: ApplicationCapability::Bidirectional,
                    evidence: vec![ApplicationLifecycleEvidence::MembershipLoss {
                        reason,
                        detail: "typed loss".into(),
                    }],
                },
            );
        }
        client.lifecycle_observations.lock().unwrap().insert(
            "collision".into(),
            LifecycleObservation {
                capability: ApplicationCapability::SendOnly,
                evidence: vec![ApplicationLifecycleEvidence::Collision(CollisionEvidence {
                    address: "collision".into(),
                    owner_instance_id: Some("owner".into()),
                    lease_epoch: Some(4),
                    guidance: "wait".into(),
                })],
            },
        );
        client.lifecycle_observations.lock().unwrap().insert(
            "compensation".into(),
            LifecycleObservation {
                capability: ApplicationCapability::SendOnly,
                evidence: vec![ApplicationLifecycleEvidence::CompensationPending(
                    CompensationHandle {
                        address: "compensation".into(),
                        runtime_id: client.runtime_id.clone(),
                        action: CompensationAction::Detach,
                    },
                )],
            },
        );
        backend
            .record_application_detach_intent(
                "application",
                "detached",
                "prior-runtime",
                "send-only",
                "ApplicationDetach",
            )
            .await
            .unwrap();

        let health = client.health().await.unwrap();
        let evidence: Vec<_> = health
            .iter()
            .flat_map(|record| record.lifecycle.iter())
            .collect();
        for reason in [
            MembershipLossReason::DaemonRestart,
            MembershipLossReason::PredicateDeath,
            MembershipLossReason::OwnerDemoted,
        ] {
            assert!(evidence.iter().any(|item| matches!(
                item,
                ApplicationLifecycleEvidence::MembershipLoss { reason: actual, .. }
                    if actual == &reason
            )));
        }
        assert!(evidence.iter().any(|item| matches!(
            item,
            ApplicationLifecycleEvidence::MembershipLoss {
                reason: MembershipLossReason::Unknown { raw_reason: Some(raw) },
                ..
            } if raw == "future-loss"
        )));
        assert!(evidence
            .iter()
            .any(|item| matches!(item, ApplicationLifecycleEvidence::Collision(_))));
        assert!(evidence
            .iter()
            .any(|item| matches!(item, ApplicationLifecycleEvidence::CompensationPending(_))));
        assert!(evidence.iter().any(|item| matches!(
            item,
            ApplicationLifecycleEvidence::DeliberateDetach {
                runtime_id: RuntimeId(runtime),
                ..
            } if runtime == "prior-runtime"
        )));
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn partial_multi_address_outcome_exposes_typed_compensation() {
        let (client, _) = sqlite_client("multi-address-outcome", "application").await;
        let handle = MembershipHandle {
            logical_store_id: client.logical_store_id.clone(),
            responsibility: client.responsibility.clone(),
            runtime_id: client.runtime_id.clone(),
            address: "attached".into(),
            capability: ApplicationCapability::SendOnly,
            lease_epoch: 1,
            owner_instance_id: "owner".into(),
        };
        client.memberships.lock().unwrap().insert(
            "attached".into(),
            LocalMembership {
                handle: handle.clone(),
                spec: AddressSpec {
                    address: "attached".into(),
                    capability: ApplicationCapability::SendOnly,
                    description: None,
                    scope: None,
                    tags: None,
                },
                recovering: false,
                last_recovery_failure: None,
            },
        );
        let outcome = client.finish_multi_address_outcome(
            BTreeMap::from([
                (
                    "attached".into(),
                    AddressLifecycleResult::Reconciled(handle),
                ),
                (
                    "failed".into(),
                    AddressLifecycleResult::Failed(ApplicationClientError::Unavailable(
                        "injected".into(),
                    )),
                ),
            ]),
            vec![CompensationHandle {
                address: "attached".into(),
                runtime_id: client.runtime_id.clone(),
                action: CompensationAction::Detach,
            }],
            None,
        );
        assert!(!outcome.ready);
        assert!(matches!(
            outcome.compensation[0].action,
            CompensationAction::Detach
        ));
        assert!(client
            .lifecycle_observations
            .lock()
            .unwrap()
            .get("attached")
            .unwrap()
            .evidence
            .iter()
            .any(|evidence| matches!(
                evidence,
                ApplicationLifecycleEvidence::CompensationPending(_)
            )));
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn lifecycle_cancellation_partitions_progress_for_every_operation_kind() {
        let (client, _) = sqlite_client("lifecycle-cancellation", "application").await;
        let specs = ["completed", "in-flight", "untouched"]
            .into_iter()
            .map(|address| AddressSpec {
                address: address.into(),
                capability: ApplicationCapability::SendOnly,
                description: None,
                scope: None,
                tags: None,
            })
            .collect::<Vec<_>>();
        let handle = MembershipHandle {
            logical_store_id: client.logical_store_id.clone(),
            responsibility: client.responsibility.clone(),
            runtime_id: client.runtime_id.clone(),
            address: "completed".into(),
            capability: ApplicationCapability::SendOnly,
            lease_epoch: 1,
            owner_instance_id: "owner".into(),
        };

        for action in [
            LifecycleAction::Attach,
            LifecycleAction::Reconcile(RecoveryPolicy::BoundedRepair { retries: 1 }),
            LifecycleAction::Detach,
        ] {
            let previous = (matches!(action, LifecycleAction::Detach)).then(|| LocalMembership {
                handle: handle.clone(),
                spec: specs[0].clone(),
                recovering: false,
                last_recovery_failure: None,
            });
            if let Some(previous) = &previous {
                client
                    .memberships
                    .lock()
                    .unwrap()
                    .insert(previous.spec.address.clone(), previous.clone());
            }
            let addresses = specs
                .iter()
                .map(|spec| spec.address.clone())
                .collect::<Vec<_>>();
            let mut operation = match action {
                LifecycleAction::Attach => client.begin_attach(&specs),
                LifecycleAction::Reconcile(policy) => client.begin_reconcile_many(&specs, policy),
                LifecycleAction::Detach => client.begin_detach_many(&addresses),
            };
            let result = match action {
                LifecycleAction::Attach => AddressLifecycleResult::Attached(handle.clone()),
                LifecycleAction::Reconcile(_) => AddressLifecycleResult::Reconciled(handle.clone()),
                LifecycleAction::Detach => AddressLifecycleResult::Detached(handle.clone()),
            };
            operation.in_flight = Some("completed".into());
            operation.record_completed(specs[0].clone(), previous, Ok(result));
            operation.in_flight = Some("in-flight".into());

            let outcome = operation.cancelled_outcome();
            assert!(!outcome.ready);
            assert_eq!(
                outcome.results.keys().cloned().collect::<Vec<_>>(),
                ["completed"]
            );
            assert_eq!(outcome.compensation.len(), 1);
            assert_eq!(
                outcome.compensation[0].action,
                match action {
                    LifecycleAction::Detach => {
                        CompensationAction::Reattach(specs[0].clone())
                    }
                    LifecycleAction::Attach | LifecycleAction::Reconcile(_) => {
                        CompensationAction::Detach
                    }
                }
            );
            assert_eq!(
                outcome.cancellation,
                Some(LifecycleCancellationEvidence {
                    operation: match action {
                        LifecycleAction::Attach => LifecycleOperationKind::Attach,
                        LifecycleAction::Reconcile(_) => LifecycleOperationKind::Reconcile,
                        LifecycleAction::Detach => LifecycleOperationKind::Detach,
                    },
                    may_have_committed: Some("in-flight".into()),
                    not_attempted: vec!["untouched".into()],
                })
            );
        }
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn reconcile_compensation_distinguishes_changed_idempotent_and_failed_work() {
        let (client, _) = sqlite_client("reconcile-compensation", "application").await;
        let previous_spec = AddressSpec {
            address: "changed".into(),
            capability: ApplicationCapability::SendOnly,
            description: Some("before".into()),
            scope: None,
            tags: None,
        };
        let changed_spec = AddressSpec {
            description: Some("after".into()),
            ..previous_spec.clone()
        };
        let untouched_spec = AddressSpec {
            address: "untouched".into(),
            ..changed_spec.clone()
        };
        let previous = LocalMembership {
            handle: MembershipHandle {
                logical_store_id: client.logical_store_id.clone(),
                responsibility: client.responsibility.clone(),
                runtime_id: client.runtime_id.clone(),
                address: previous_spec.address.clone(),
                capability: previous_spec.capability,
                lease_epoch: 1,
                owner_instance_id: "owner".into(),
            },
            spec: previous_spec.clone(),
            recovering: false,
            last_recovery_failure: None,
        };
        client
            .memberships
            .lock()
            .unwrap()
            .insert(previous_spec.address.clone(), previous.clone());
        let reconciled = MembershipHandle {
            capability: changed_spec.capability,
            ..previous.handle.clone()
        };

        let specs = [changed_spec.clone(), untouched_spec.clone()];
        let mut changed =
            client.begin_reconcile_many(&specs, RecoveryPolicy::BoundedRepair { retries: 1 });
        changed.record_completed(
            changed_spec.clone(),
            Some(previous.clone()),
            Ok(AddressLifecycleResult::Reconciled(reconciled.clone())),
        );
        let changed_outcome = changed.cancelled_outcome();
        assert_eq!(
            changed_outcome.compensation,
            [CompensationHandle {
                address: changed_spec.address.clone(),
                runtime_id: client.runtime_id.clone(),
                action: CompensationAction::Reattach(previous_spec.clone()),
            }]
        );
        assert!(client
            .lifecycle_observations
            .lock()
            .unwrap()
            .get(&changed_spec.address)
            .unwrap()
            .evidence
            .iter()
            .any(|evidence| matches!(
                evidence,
                ApplicationLifecycleEvidence::CompensationPending(_)
            )));

        let mut idempotent = client.begin_reconcile_many(
            &[previous_spec.clone(), untouched_spec.clone()],
            RecoveryPolicy::BoundedRepair { retries: 1 },
        );
        idempotent.record_completed(
            previous_spec.clone(),
            Some(previous.clone()),
            Ok(AddressLifecycleResult::Reconciled(previous.handle.clone())),
        );
        assert!(idempotent.cancelled_outcome().compensation.is_empty());

        let mut strict = client.begin_reconcile_many(&specs, RecoveryPolicy::Strict);
        strict.record_completed(
            changed_spec.clone(),
            Some(previous.clone()),
            Ok(AddressLifecycleResult::Reconciled(reconciled)),
        );
        assert!(strict.cancelled_outcome().compensation.is_empty());

        let mut failed =
            client.begin_reconcile_many(&specs, RecoveryPolicy::BoundedRepair { retries: 1 });
        failed.record_completed(
            changed_spec.clone(),
            Some(previous),
            Err(ApplicationClientError::InvalidRequest(
                "injected reconcile failure".into(),
            )),
        );
        let failed_outcome = failed.cancelled_outcome();
        assert!(matches!(
            failed_outcome.results.get(&changed_spec.address),
            Some(AddressLifecycleResult::Failed(
                ApplicationClientError::InvalidRequest(_)
            ))
        ));
        assert!(failed_outcome.compensation.is_empty());
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn cancelling_polled_reconcile_run_retains_uncertainty_and_health() {
        let (client, _) = sqlite_client("reconcile-midflight-cancellation", "application").await;
        let specs = ["in-flight", "untouched"]
            .into_iter()
            .map(|address| AddressSpec {
                address: address.into(),
                capability: ApplicationCapability::SendOnly,
                description: None,
                scope: None,
                tags: None,
            })
            .collect::<Vec<_>>();
        let gate = TestGate {
            started: Arc::new(tokio::sync::Notify::new()),
            release: Arc::new(tokio::sync::Notify::new()),
        };
        let mut operation =
            client.begin_reconcile_many(&specs, RecoveryPolicy::BoundedRepair { retries: 1 });
        operation.test_gate = Some(gate.clone());

        {
            let run = operation.run();
            tokio::pin!(run);
            tokio::select! {
                _ = gate.started.notified() => {}
                outcome = &mut run => {
                    panic!("lifecycle run completed before cancellation gate: {outcome:?}");
                }
            }
        }

        let outcome = operation.cancelled_outcome();
        assert!(!outcome.ready);
        assert_eq!(
            outcome.cancellation,
            Some(LifecycleCancellationEvidence {
                operation: LifecycleOperationKind::Reconcile,
                may_have_committed: Some("in-flight".into()),
                not_attempted: vec!["untouched".into()],
            })
        );
        let recovery = client
            .recovery_attempts
            .lock()
            .unwrap()
            .get("in-flight")
            .cloned()
            .unwrap();
        assert!(recovery.recovering);
        let health = client.health().await.unwrap();
        let in_flight = health
            .iter()
            .find(|record| record.address == "in-flight")
            .unwrap();
        assert!(in_flight.recovering);
        assert!(in_flight.degraded);
        assert!(in_flight.lifecycle.iter().any(|evidence| matches!(
            evidence,
                ApplicationLifecycleEvidence::Reconciliation {
                    state: ReconciliationEvidence::InProgress,
                    detail: Some(detail),
                } if detail.contains("canceled without a terminal outcome")
        )));
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn lifecycle_cancellation_before_work_marks_every_address_not_attempted() {
        let (client, _) = sqlite_client("lifecycle-cancellation-before-work", "application").await;
        let specs = ["first", "second"]
            .into_iter()
            .map(|address| AddressSpec {
                address: address.into(),
                capability: ApplicationCapability::SendOnly,
                description: None,
                scope: None,
                tags: None,
            })
            .collect::<Vec<_>>();

        let outcome = client.begin_attach(&specs).cancelled_outcome();
        assert!(!outcome.ready);
        assert!(outcome.results.is_empty());
        assert!(outcome.compensation.is_empty());
        assert_eq!(
            outcome.cancellation,
            Some(LifecycleCancellationEvidence {
                operation: LifecycleOperationKind::Attach,
                may_have_committed: None,
                not_attempted: vec!["first".into(), "second".into()],
            })
        );
    }

    #[test]
    fn attach_compensation_preserves_preexisting_membership() {
        let prior_spec = AddressSpec {
            address: "attached".into(),
            capability: ApplicationCapability::SendOnly,
            description: Some("prior".into()),
            scope: None,
            tags: None,
        };
        let previous = LocalMembership {
            handle: MembershipHandle {
                logical_store_id: LogicalStoreId("store".into()),
                responsibility: ApplicationResponsibility("application".into()),
                runtime_id: RuntimeId("runtime".into()),
                address: prior_spec.address.clone(),
                capability: prior_spec.capability,
                lease_epoch: 1,
                owner_instance_id: "owner".into(),
            },
            spec: prior_spec.clone(),
            recovering: false,
            last_recovery_failure: None,
        };

        assert_eq!(attach_compensation(Some(&previous), &prior_spec), None);
        let changed = AddressSpec {
            description: Some("changed".into()),
            ..prior_spec.clone()
        };
        assert_eq!(
            attach_compensation(Some(&previous), &changed),
            Some(CompensationAction::Reattach(prior_spec))
        );
        assert_eq!(
            attach_compensation(None, &changed),
            Some(CompensationAction::Detach)
        );
    }

    #[test]
    fn payload_identity_marks_noncanonical_evidence_noncomparable() {
        assert!(PayloadIdentity::sha256("a".repeat(64)).comparable);
        assert!(!PayloadIdentity::sha256("legacy-opaque".into()).comparable);
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
        assert_eq!(
            project_membership_loss(
                Some(NeedsAttachReason::PredicateDeath),
                crate::daemon_ipc::ERROR_NEEDS_ATTACH,
                "predicate"
            ),
            MembershipLossReason::PredicateDeath
        );
        assert_eq!(
            project_membership_loss(
                Some(NeedsAttachReason::Unknown("future_reason".into())),
                crate::daemon_ipc::ERROR_NEEDS_ATTACH,
                "fallback text"
            ),
            MembershipLossReason::Unknown {
                raw_reason: Some("future_reason".into())
            }
        );
    }

    #[test]
    fn unknown_wire_membership_reason_preserves_exact_token() {
        let reason: NeedsAttachReason = serde_json::from_str("\"future_reason\"").unwrap();
        assert_eq!(
            reason,
            NeedsAttachReason::Unknown("future_reason".to_string())
        );
        assert_eq!(serde_json::to_string(&reason).unwrap(), "\"future_reason\"");
        assert_eq!(
            project_membership_loss(
                Some(reason),
                crate::daemon_ipc::ERROR_NEEDS_ATTACH,
                "fallback text"
            ),
            MembershipLossReason::Unknown {
                raw_reason: Some("future_reason".to_string())
            }
        );
    }

    #[test]
    fn peer_failure_allowlist_preserves_acceptance_uncertainty() {
        assert!(matches!(
            classify_peer_failure(crate::daemon_ipc::ERROR_INCOMPATIBLE, None),
            PeerFailureDisposition::Rejected
        ));
        assert!(matches!(
            classify_peer_failure(
                crate::daemon_ipc::ERROR_NEEDS_ATTACH,
                Some(NeedsAttachReason::RestartLost)
            ),
            PeerFailureDisposition::NeedsAttach
        ));
        assert!(matches!(
            classify_peer_failure(crate::daemon_ipc::ERROR_INTERNAL, None),
            PeerFailureDisposition::Indeterminate
        ));
        assert!(matches!(
            classify_peer_failure("FuturePeerError", None),
            PeerFailureDisposition::Indeterminate
        ));
    }

    #[test]
    fn preacceptance_rejection_exposes_typed_retryability() {
        let transient = preaccept_rejection(
            crate::daemon_ipc::ERROR_NOT_RUNNING,
            "daemon draining",
            RejectionRetryability::Transient,
        );
        let permanent = preaccept_rejection(
            crate::daemon_ipc::ERROR_INCOMPATIBLE,
            "recipient retired",
            RejectionRetryability::Permanent,
        );
        match transient {
            ApplicationClientError::RejectedBeforeAcceptance {
                retryability: RejectionRetryability::Transient,
                detail,
                ..
            } => assert!(!detail.contains("daemon draining")),
            other => panic!("expected transient rejection, got {other:?}"),
        }
        assert!(matches!(
            permanent,
            ApplicationClientError::RejectedBeforeAcceptance {
                retryability: RejectionRetryability::Permanent,
                ..
            }
        ));
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
        assert_eq!(
            reason,
            NeedsAttachReason::Unknown("future_reason".to_string())
        );
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
        let payload_fingerprint = "a".repeat(64);
        backend
            .begin_application_operation(&NewApplicationOperation {
                logical_store_id: logical_store_id.0.clone(),
                application_responsibility: "watcher".into(),
                operation_id: "op-crash-window".into(),
                operation_kind: "send".into(),
                sender: "watcher:sender".into(),
                recipients_json: r#"["target"]"#.into(),
                payload_fingerprint: payload_fingerprint.clone(),
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
                    logical_store_id: logical_store_id.0.clone(),
                    application_responsibility: "watcher".into(),
                    operation_id: "op-crash-window".into(),
                    payload_fingerprint: payload_fingerprint.clone(),
                },
            )
            .await
            .unwrap();
        let current_client = ApplicationClient {
            responsibility: ApplicationResponsibility("watcher".into()),
            runtime_id: RuntimeId::fresh().unwrap(),
            logical_store_id: logical_store_id.clone(),
            store_key: store_key.clone(),
            profile: profile.clone(),
            backend: backend.clone(),
            memberships: Mutex::new(BTreeMap::new()),
            outstanding_acks: Mutex::new(BTreeSet::new()),
            recovery_attempts: Mutex::new(BTreeMap::new()),
            lifecycle_observations: Mutex::new(BTreeMap::new()),
        };
        let current_noncomparable = RecoveryHandle {
            logical_store_id: logical_store_id.clone(),
            responsibility: ApplicationResponsibility("watcher".into()),
            operation_id: OperationId("op-crash-window".into()),
            payload_identity: PayloadIdentity {
                algorithm: "sha256".into(),
                digest: payload_fingerprint.clone(),
                comparable: false,
            },
            retention_generation: None,
        };
        assert!(matches!(
            current_client
                .reconcile_operation(&current_noncomparable)
                .await,
            Err(ApplicationClientError::OperationMismatch { .. })
        ));
        assert_eq!(
            backend
                .application_operation(&logical_store_id.0, "watcher", "op-crash-window")
                .await
                .unwrap()
                .unwrap()
                .state,
            "pending"
        );
        drop(current_client);
        drop(backend);

        let profile = crate::profiles::implicit_sqlite(Some(&path));
        let store_key = crate::profiles::store_key(&profile, Some(&path));
        let backend = Arc::new(SqliteBackend::open(&path).unwrap());
        backend.init_schema().await.unwrap();
        let reopened_store_id =
            LogicalStoreId::persisted(backend.logical_store_id().await.unwrap());
        assert_eq!(reopened_store_id, logical_store_id);
        let client = ApplicationClient {
            responsibility: ApplicationResponsibility("watcher".into()),
            runtime_id: RuntimeId::fresh().unwrap(),
            logical_store_id: reopened_store_id,
            store_key,
            profile,
            backend: backend.clone(),
            memberships: Mutex::new(BTreeMap::new()),
            outstanding_acks: Mutex::new(BTreeSet::new()),
            recovery_attempts: Mutex::new(BTreeMap::new()),
            lifecycle_observations: Mutex::new(BTreeMap::new()),
        };
        let mismatched_reference = RecoveryHandle {
            logical_store_id: LogicalStoreId::persisted("store-v1-other".into()),
            responsibility: ApplicationResponsibility("watcher".into()),
            operation_id: OperationId("op-crash-window".into()),
            payload_identity: PayloadIdentity::sha256(payload_fingerprint.clone()),
            retention_generation: None,
        };
        assert!(matches!(
            client.reconcile_operation(&mismatched_reference).await,
            Err(ApplicationClientError::StoreBindingMismatch { .. })
        ));

        let noncomparable_reference = RecoveryHandle {
            logical_store_id: logical_store_id.clone(),
            responsibility: ApplicationResponsibility("watcher".into()),
            operation_id: OperationId("op-crash-window".into()),
            payload_identity: PayloadIdentity {
                algorithm: "sha256".into(),
                digest: payload_fingerprint.clone(),
                comparable: false,
            },
            retention_generation: None,
        };
        assert!(matches!(
            client.reconcile_operation(&noncomparable_reference).await,
            Err(ApplicationClientError::OperationMismatch { .. })
        ));
        assert_eq!(
            backend
                .application_operation(&logical_store_id.0, "watcher", "op-crash-window")
                .await
                .unwrap()
                .unwrap()
                .state,
            "pending"
        );
        let reference = client
            .operation_reference(
                OperationId("op-crash-window".into()),
                PayloadIdentity::sha256(payload_fingerprint),
            )
            .await
            .unwrap();
        let OperationReconciliation::Recorded(reconciled) =
            client.reconcile_operation(&reference).await.unwrap()
        else {
            panic!("expected recorded operation");
        };
        let RecordedOperationOutcome::Accepted(result) = reconciled.outcome else {
            panic!("expected typed accepted outcome");
        };
        assert_eq!(result.recipient, "target");
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn not_recorded_is_authoritative_only_within_retention_generation() {
        let path = std::env::temp_dir()
            .join(format!(
                "telex-application-not-recorded-{}-{}.db",
                std::process::id(),
                now_ms()
            ))
            .to_string_lossy()
            .into_owned();
        let profile = crate::profiles::implicit_sqlite(Some(&path));
        let backend = Arc::new(SqliteBackend::open(&path).unwrap());
        backend.init_schema().await.unwrap();
        let client = ApplicationClient {
            responsibility: ApplicationResponsibility("watcher".into()),
            runtime_id: RuntimeId::fresh().unwrap(),
            logical_store_id: LogicalStoreId::persisted(backend.logical_store_id().await.unwrap()),
            store_key: crate::profiles::store_key(&profile, Some(&path)),
            profile,
            backend: backend.clone(),
            memberships: Mutex::new(BTreeMap::new()),
            outstanding_acks: Mutex::new(BTreeSet::new()),
            recovery_attempts: Mutex::new(BTreeMap::new()),
            lifecycle_observations: Mutex::new(BTreeMap::new()),
        };
        let reference = client
            .operation_reference(
                OperationId("never-submitted".into()),
                PayloadIdentity::sha256("a".repeat(64)),
            )
            .await
            .unwrap();
        assert!(matches!(
            client.reconcile_operation(&reference).await.unwrap(),
            OperationReconciliation::NotRecorded(NotRecordedEvidence {
                operation_id: OperationId(ref operation_id),
                retention_generation: 0,
                ..
            }) if operation_id == "never-submitted"
        ));

        let other_terminal = NewApplicationOperation {
            logical_store_id: client.logical_store_id.0.clone(),
            application_responsibility: "other-app".into(),
            operation_id: "other-cleanup-boundary".into(),
            operation_kind: "send".into(),
            sender: "other".into(),
            recipients_json: "[]".into(),
            payload_fingerprint: "c".repeat(64),
            retry_budget: 0,
            created_at_ms: 1,
        };
        backend
            .begin_application_operation(&other_terminal)
            .await
            .unwrap();
        backend
            .complete_application_operation(
                &other_terminal.logical_store_id,
                &other_terminal.application_responsibility,
                &other_terminal.operation_id,
                "rejected",
                Some("{}"),
                None,
            )
            .await
            .unwrap();
        backend
            .cleanup_application_records(
                &ApplicationRecordScope {
                    logical_store_id: other_terminal.logical_store_id.clone(),
                    application_responsibility: other_terminal.application_responsibility.clone(),
                },
                RetentionPolicy {
                    completed_before_ms: i64::MAX,
                    max_delete: 1,
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            client.reconcile_operation(&reference).await.unwrap(),
            OperationReconciliation::NotRecorded(_)
        ));

        let terminal = NewApplicationOperation {
            logical_store_id: client.logical_store_id.0.clone(),
            application_responsibility: client.responsibility.0.clone(),
            operation_id: "cleanup-boundary".into(),
            operation_kind: "send".into(),
            sender: "watcher".into(),
            recipients_json: "[]".into(),
            payload_fingerprint: "b".repeat(64),
            retry_budget: 0,
            created_at_ms: 1,
        };
        backend
            .begin_application_operation(&terminal)
            .await
            .unwrap();
        backend
            .complete_application_operation(
                &terminal.logical_store_id,
                &terminal.application_responsibility,
                &terminal.operation_id,
                "rejected",
                Some("{}"),
                None,
            )
            .await
            .unwrap();
        client
            .cleanup(RetentionPolicy {
                completed_before_ms: i64::MAX,
                max_delete: 1,
            })
            .await
            .unwrap();
        assert!(matches!(
            client.reconcile_operation(&reference).await.unwrap(),
            OperationReconciliation::RetentionBoundaryCrossed {
                staged_generation: Some(0),
                current_generation: 1,
            }
        ));
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
            lifecycle_observations: Mutex::new(BTreeMap::new()),
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
    async fn receipt_refresh_preserves_indeterminate_recovery_evidence() {
        let path = std::env::temp_dir()
            .join(format!(
                "telex-application-receipt-indeterminate-{}-{}.db",
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
            lifecycle_observations: Mutex::new(BTreeMap::new()),
        };
        let operation_id = OperationId("indeterminate-peer-error".into());
        let payload_fingerprint = "b".repeat(64);
        backend
            .begin_application_operation(&NewApplicationOperation {
                logical_store_id: logical_store_id.0.clone(),
                application_responsibility: "watcher".into(),
                operation_id: operation_id.0.clone(),
                operation_kind: "send".into(),
                sender: "watcher:sender".into(),
                recipients_json: r#"["target"]"#.into(),
                payload_fingerprint: payload_fingerprint.clone(),
                retry_budget: 1,
                created_at_ms: now_ms(),
            })
            .await
            .unwrap();
        let peer_error = ApplicationClientError::Unavailable("peer internal".into());
        let recovery = client
            .operation_reference(
                operation_id.clone(),
                PayloadIdentity::sha256(payload_fingerprint),
            )
            .await
            .unwrap();
        backend
            .complete_application_operation(
                &logical_store_id.0,
                "watcher",
                &operation_id.0,
                "indeterminate",
                Some(&serde_json::to_string(&peer_error).unwrap()),
                Some(&serde_json::to_string(&recovery).unwrap()),
            )
            .await
            .unwrap();
        match client.refresh_receipt_axes(&recovery).await {
            Err(ApplicationClientError::Indeterminate {
                recovery: actual, ..
            }) => assert_eq!(actual.operation_id, operation_id),
            other => panic!("expected indeterminate receipt refresh, got {other:?}"),
        }
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
        let reconcile_duplicate = client
            .reconcile_many(
                &[
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
                ],
                RecoveryPolicy::BoundedRepair { retries: 1 },
            )
            .await;
        assert!(!reconcile_duplicate.ready);
        assert!(reconcile_duplicate.validation_error.is_some());
        let detach_duplicate = client
            .detach_many(&["watcher:sender".into(), "watcher:sender".into()])
            .await;
        assert!(!detach_duplicate.ready);
        assert!(detach_duplicate.validation_error.is_some());
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
                    spec: AddressSpec {
                        address: "station:inbox".into(),
                        capability: ApplicationCapability::Bidirectional,
                        description: None,
                        scope: None,
                        tags: None,
                    },
                    recovering: false,
                    last_recovery_failure: None,
                },
            )])),
            outstanding_acks: Mutex::new(BTreeSet::new()),
            recovery_attempts: Mutex::new(BTreeMap::new()),
            lifecycle_observations: Mutex::new(BTreeMap::new()),
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
        let mut local = client
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

        local.handle.capability = ApplicationCapability::SendOnly;
        client
            .memberships
            .lock()
            .unwrap()
            .insert("station:inbox".into(), local.clone());
        assert!(matches!(
            client
                .history(Some("station:inbox".into()), true, None, None, None, 10)
                .await,
            Err(ApplicationClientError::UnsupportedCapability(_))
        ));
        let member: MemberStatus = serde_json::from_value(serde_json::json!({
            "store_key": "sqlite:test",
            "backend": "sqlite",
            "session_id": "runtime",
            "address": "station:inbox",
            "occupant": "runtime",
            "host": "test",
            "waiters": 0,
            "pending_unconsumed_count": 3,
            "inbound_actionable_count": 2,
            "health_detail": "2 actionable inbound messages",
            "deaf_warn": true,
            "lease_epoch": 1,
            "owner_instance_id": "owner",
            "idle": false
        }))
        .unwrap();
        let send_only_health = health_projection(&client, &local, Some(&member));
        assert_eq!(send_only_health.pending_unconsumed, 0);
        assert_eq!(send_only_health.inbound_actionable, 0);
        assert!(!send_only_health.attended_but_deaf);
        assert!(send_only_health.evidence.is_empty());
        assert!(!send_only_health.degraded);
    }
}
