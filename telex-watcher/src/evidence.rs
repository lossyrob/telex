//! Versioned local evidence. Detector state stays in the registry, never in messages.

use crate::protocol_v2::{DetectorEvent, Registration};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use telex::application_client::{
    ApplicationClientError, ApplicationHealth, MultiAddressOutcome, OperationReconciliation,
    RecoveryHandle, SendRequest, SendResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Lifecycle {
    Active,
    Paused,
    Terminal,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Eligibility {
    Eligible,
    ReconciliationPending,
    Inactive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Health {
    Ready,
    Degraded,
    Blocked,
    Inactive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlockedReason {
    ReceiptOutcomeUnresolved,
    SendRetryExhausted,
    SendPermanentlyRejected,
    OperationIdentityConflict,
    LogicalStoreMismatch,
    UnsupportedRegistrationVersion,
    OrphanContainmentUnproven,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiagnosticCategory {
    DetectorDegraded,
    ProcessFailure,
    MalformedOutput,
    CanonicalizationFailed,
    OutputLimit,
    Timeout,
    SendFailure,
    UnknownReceipt,
    ReceiptReconciliationPending,
    RuntimeInterrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Recurrence {
    pub last_recurrence_hash: Option<String>,
    pub consecutive_identical_recurrences: u64,
    pub first_identical_at: Option<String>,
    pub last_identical_at: Option<String>,
    pub last_identical_event_sequence: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Watch {
    pub schema_version: u32,
    pub id: String,
    pub registration: Option<Registration>,
    pub registration_revision: u64,
    pub lifecycle: Lifecycle,
    pub eligibility: Eligibility,
    pub health: Health,
    pub blocked_reason: Option<BlockedReason>,
    pub activated_at: String,
    pub last_success_at: Option<String>,
    pub last_event_at: Option<String>,
    pub last_attempt_id: Option<String>,
    pub running_attempt_id: Option<String>,
    pub running_cancelled: bool,
    pub state: Value,
    pub logical_store_id: Option<String>,
    #[serde(default)]
    pub observed_logical_store_id: Option<String>,
    pub last_allocated_event_sequence: u64,
    pub last_committed_event_sequence: u64,
    pub pending_operation_id: Option<String>,
    pub next_due_ms: i64,
    pub next_attempt_ms: i64,
    pub suppressed_due_count: u64,
    pub coalesced_due: bool,
    pub consecutive_failures: u32,
    pub latest_attempt_duration_ms: Option<u64>,
    pub diagnostic_category: Option<DiagnosticCategory>,
    pub recurrence: Recurrence,
}

impl Watch {
    pub(crate) fn block(&mut self, reason: BlockedReason) {
        if matches!(self.lifecycle, Lifecycle::Active | Lifecycle::Paused) {
            self.lifecycle = Lifecycle::Paused;
            self.eligibility = Eligibility::Inactive;
            self.health = Health::Blocked;
            self.blocked_reason = Some(reason);
        }
    }

    pub(crate) fn store_verified(&self) -> bool {
        self.logical_store_id
            .as_deref()
            .is_none_or(|bound| self.observed_logical_store_id.as_deref() == Some(bound))
    }

    pub(crate) fn pending(&mut self) {
        if self.lifecycle == Lifecycle::Active {
            self.eligibility = Eligibility::ReconciliationPending;
            self.health = Health::Degraded;
            self.blocked_reason = None;
            self.diagnostic_category = Some(DiagnosticCategory::ReceiptReconciliationPending);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Attempt {
    pub schema_version: u32,
    pub id: String,
    pub watch_id: String,
    pub runtime_id: String,
    pub registration_revision: u64,
    pub started_at: String,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub command: Vec<String>,
    pub working_directory: std::path::PathBuf,
    pub prior_state_hash: String,
    pub outcome: AttemptOutcome,
    pub diagnostic_category: Option<DiagnosticCategory>,
    pub diagnostic: Option<String>,
    pub containment: Option<Value>,
    pub state_committed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AttemptOutcome {
    Running,
    Idle,
    Terminal,
    Pending,
    Accepted,
    Failed,
    Interrupted,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperationStatus {
    Staged,
    QueryRequired,
    RetryPermitted,
    Accepted,
    ClosedNotAccepted,
    RemovedNotRecorded,
    RemovedRejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Budget {
    pub remaining: u32,
    pub used: u32,
    pub deadline_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Operation {
    pub schema_version: u32,
    pub watch_id: String,
    pub event_sequence: u64,
    pub event_id: String,
    pub operation_id: String,
    pub attempt_id: String,
    pub attempt_now: String,
    pub runtime_id: String,
    pub registration_revision: u64,
    pub registration: Registration,
    pub prior_state: Value,
    pub proposed_state: Value,
    pub terminal: bool,
    pub envelope: SendRequest,
    pub recovery: RecoveryHandle,
    pub prior_state_hash: String,
    pub proposed_state_hash: String,
    pub envelope_hash: String,
    pub recurrence_hash: String,
    pub hash_algorithm: String,
    pub hash_preimage_version: u32,
    pub status: OperationStatus,
    pub query_only: bool,
    pub reconciliation_budget: Budget,
    pub send_budget: Budget,
    pub next_recovery_ms: i64,
    pub last_evidence_at: Option<String>,
    pub last_evidence: Option<SendEvidence>,
    pub receipt: Option<SendResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "source", content = "evidence")]
pub enum SendEvidence {
    Send(Result<Box<SendResult>, ApplicationClientError>),
    Reconciliation(Result<OperationReconciliation, ApplicationClientError>),
    ObservationCancelled,
}

impl SendEvidence {
    pub(crate) fn send(result: Result<SendResult, ApplicationClientError>) -> Self {
        Self::Send(result.map(Box::new))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceiptEvidence {
    pub schema_version: u32,
    pub operation_id: String,
    pub observed_at: String,
    pub evidence: SendEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventMetadata {
    pub schema_version: u32,
    pub watcher: WatcherProvenance,
    pub detector: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WatcherProvenance {
    pub watch_id: String,
    pub event_sequence: u64,
    pub event_id: String,
    pub operation_id: String,
    pub attempt_id: String,
    pub runtime_id: String,
    pub logical_store_id: String,
    pub registration_revision: u64,
    pub detector_schema_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Retention {
    pub rows: u64,
    pub bytes: u64,
    pub warning_rows: u64,
    pub warning_bytes: u64,
    pub warning: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SenderHealth {
    pub backend_profile: String,
    #[serde(default)]
    pub required_senders: Vec<String>,
    pub logical_store_id: Option<String>,
    pub ready: bool,
    pub membership: Vec<ApplicationHealth>,
    pub lifecycle: Option<MultiAddressOutcome>,
    pub compensation: Vec<MultiAddressOutcome>,
    pub error: Option<ApplicationClientError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeHealth {
    pub schema_version: u32,
    pub runtime_id: String,
    pub pid: u32,
    pub started_at: String,
    pub heartbeat_at: String,
    pub status: RuntimeStatus,
    pub production_ready: bool,
    pub registry_revision: u64,
    #[serde(default)]
    pub reconciled_registry_revision: Option<u64>,
    pub restart_reconciled: bool,
    pub senders: Vec<SenderHealth>,
    pub retention: Retention,
    pub diagnostics: Vec<RuntimeDiagnostic>,
    pub detach: Vec<MultiAddressOutcome>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeStatus {
    Running,
    Stopped,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeDiagnostic {
    MembershipReconciliationPending,
    SenderUnready,
    PartialMembership,
    InboundActionableBacklog,
    RetentionWarning,
    ContainmentUnproven,
    RegistryFailure,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WatchHealth {
    pub schema_version: u32,
    pub id: String,
    pub registration_revision: u64,
    pub detector_schema_version: u32,
    pub activated_at: String,
    pub lifecycle: Lifecycle,
    pub eligibility: Eligibility,
    pub health: Health,
    pub blocked_reason: Option<BlockedReason>,
    pub last_allocated_event_sequence: u64,
    pub last_committed_event_sequence: u64,
    pub pending: Option<PendingHealth>,
    pub last_attempt_id: Option<String>,
    pub last_success_at: Option<String>,
    pub last_event_at: Option<String>,
    pub next_attempt_ms: i64,
    pub latest_attempt_duration_ms: Option<u64>,
    pub suppressed_due_count: u64,
    pub consecutive_failures: u32,
    pub diagnostic_category: Option<DiagnosticCategory>,
    pub recurrence: Recurrence,
    pub sender_ready: bool,
    pub logical_store_id: Option<String>,
    pub observed_logical_store_id: Option<String>,
    pub retention: Retention,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingHealth {
    pub event_sequence: u64,
    pub event_id: String,
    pub operation_id: String,
    pub logical_store_id: String,
    pub status: OperationStatus,
    pub reconciliation_budget: Budget,
    pub send_budget: Budget,
    pub last_evidence_at: Option<String>,
}

impl From<&Operation> for PendingHealth {
    fn from(op: &Operation) -> Self {
        Self {
            event_sequence: op.event_sequence,
            event_id: op.event_id.clone(),
            operation_id: op.operation_id.clone(),
            logical_store_id: op.recovery.logical_store_id.0.clone(),
            status: op.status,
            reconciliation_budget: op.reconciliation_budget.clone(),
            send_budget: op.send_budget.clone(),
            last_evidence_at: op.last_evidence_at.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EventProposal {
    pub event: DetectorEvent,
    pub next_state: Value,
    pub terminal: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concrete_evidence_models_round_trip_the_schema_corpus_exactly() {
        let corpus: Value =
            serde_json::from_str(include_str!("../tests/fixtures/evidence-v2.json")).unwrap();
        let metadata: EventMetadata = serde_json::from_value(corpus["metadata"].clone()).unwrap();
        let runtime: RuntimeHealth = serde_json::from_value(corpus["runtime"].clone()).unwrap();
        let watch: WatchHealth = serde_json::from_value(corpus["watch"].clone()).unwrap();
        assert_eq!(serde_json::to_value(metadata).unwrap(), corpus["metadata"]);
        assert_eq!(serde_json::to_value(runtime).unwrap(), corpus["runtime"]);
        assert_eq!(serde_json::to_value(watch).unwrap(), corpus["watch"]);
    }
}
