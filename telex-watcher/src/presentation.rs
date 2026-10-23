//! Explicit allowlisted local output. Never serialize raw management responses.

use crate::cli::Filters;
use crate::evidence::*;
use crate::management::Response;
use crate::protocol_v2::{hash_value, Registration};
use crate::Refusal;
use serde::Serialize;
use serde_json::{json, Value};
use telex::application_client::{
    AddressLifecycleResult, ApplicationClientError, ApplicationHealth,
    ApplicationLifecycleEvidence, EvidenceState, MembershipLossReason, MultiAddressOutcome,
    OperationReconciliation, RecordedOperationOutcome, SendResult,
};

pub const OUTPUT_SCHEMA_VERSION: u32 = 1;
pub const HEARTBEAT_STALE_AFTER_MS: i64 = 5_000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub schema_version: u32,
    pub command: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ProjectedResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<PublicError>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedResult {
    pub r#type: &'static str,
    pub value: Value,
}

#[derive(Debug, Serialize)]
pub struct PublicError {
    pub code: String,
    pub message: String,
}

impl Envelope {
    pub fn error(command: &str, code: &str, message: &str) -> Self {
        Self {
            schema_version: OUTPUT_SCHEMA_VERSION,
            command: command.into(),
            ok: false,
            result: None,
            error: Some(PublicError {
                code: code.into(),
                message: message.into(),
            }),
        }
    }

    pub fn from_response(
        command: &str,
        response: &Response,
        filters: Option<&Filters>,
        now_ms: i64,
    ) -> Self {
        let mut error = None;
        let (kind, value) = match response {
            Response::Mutation(mutation) => {
                if let Some(refusal) = mutation.refusal {
                    error = Some(PublicError {
                        code: serde_json::to_value(refusal)
                            .expect("refusal enum serialization")
                            .as_str()
                            .expect("refusal string")
                            .to_owned(),
                        message: refusal_message(refusal).into(),
                    });
                }
                (
                    "mutation",
                    json!({
                        "watch":mutation.watch.as_ref().map(watch),
                        "refusal":mutation.refusal,
                        "diagnosticPresent":mutation.diagnostic.is_some(),
                        "recovery":mutation.recovery.as_ref().map(send_evidence),
                        "audit":mutation.audit,
                    }),
                )
            }
            Response::Watch(item) => {
                let item = item
                    .as_ref()
                    .filter(|item| filters.is_none_or(|filters| filters.matches(item)));
                if item.is_none() {
                    error = Some(PublicError {
                        code: "not-found".into(),
                        message: "no watch matches the ID and selected filters".into(),
                    });
                }
                ("watch", item.map(watch).unwrap_or(Value::Null))
            }
            Response::Watches(items) => (
                "watches",
                items
                    .iter()
                    .filter(|item| filters.is_none_or(|filters| filters.matches(item)))
                    .map(watch)
                    .collect(),
            ),
            Response::Health(health) => ("health", runtime_health(health, now_ms)),
            // WatchHealth contains only public identities, hashes, enums, counters and budgets.
            Response::WatchHealth(health) => ("watchHealth", json!(health)),
            Response::Attempts(items) => ("attempts", items.iter().map(attempt).collect()),
            Response::Events(items) => ("events", items.iter().map(operation).collect()),
            Response::Pending(item) => (
                "pending",
                item.as_deref().map(operation).unwrap_or(Value::Null),
            ),
            Response::ReceiptEvidence(items) => (
                "receiptEvidence",
                items
                    .iter()
                    .map(|item| {
                        json!({
                            "schemaVersion":item.schema_version,
                            "operationId":item.operation_id,
                            "observedAt":item.observed_at,
                            "evidence":send_evidence(&item.evidence),
                        })
                    })
                    .collect(),
            ),
            Response::Stopping => ("stopping", json!({"stopping":true})),
        };
        Self {
            schema_version: OUTPUT_SCHEMA_VERSION,
            command: command.into(),
            ok: error.is_none(),
            result: Some(ProjectedResult {
                r#type: kind,
                value,
            }),
            error,
        }
    }

    pub fn human(&self) -> String {
        let mut lines = Vec::new();
        if let Some(error) = &self.error {
            lines.push(format!(
                "{}: {}: {}",
                self.command, error.code, error.message
            ));
        }
        if let Some(result) = &self.result {
            match result.r#type {
                "mutation" => {
                    if let Some(item) = result.value.get("watch").filter(|item| !item.is_null()) {
                        lines.extend(human_watch(item));
                    }
                    for name in ["audit", "recovery"] {
                        if let Some(item) = result.value.get(name).filter(|item| !item.is_null()) {
                            lines.push(format!("{name}: {item}"));
                        }
                    }
                }
                "watch" if !result.value.is_null() => lines.extend(human_watch(&result.value)),
                "watches" => {
                    if let Some(items) = result.value.as_array() {
                        lines.push(format!("{} watch(es)", items.len()));
                        for item in items {
                            lines.extend(human_watch(item));
                        }
                    }
                }
                "attempts" | "events" | "receiptEvidence" => {
                    if let Some(items) = result.value.as_array() {
                        lines.push(format!("{} {} record(s)", items.len(), result.r#type));
                        lines.extend(items.iter().map(Value::to_string));
                    }
                }
                "stopping" => lines.push("runtime shutdown admitted".into()),
                _ => lines.push(format!("{}: {}", result.r#type, result.value)),
            }
        }
        lines.join("\n")
    }
}

fn human_watch(item: &Value) -> Vec<String> {
    let field = |name: &str| {
        item.get(name)
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or(value.to_string())
            })
            .unwrap_or_else(|| "-".into())
    };
    let mut lines = vec![format!(
        "{}: {} / {} / {} revision={} reason={} pending={} sequences={}/{}",
        field("id"),
        field("lifecycle"),
        field("eligibility"),
        field("health"),
        field("registrationRevision"),
        field("blockedReason"),
        field("pendingOperationId"),
        field("lastCommittedEventSequence"),
        field("lastAllocatedEventSequence"),
    )];
    if let Some(policy) = item.get("registration").filter(|value| !value.is_null()) {
        lines.push(format!("  effective policy: {policy}"));
    }
    if let Some(category) = item
        .get("diagnosticCategory")
        .filter(|value| !value.is_null())
    {
        lines.push(format!("  diagnostic: {category}"));
    }
    lines
}

fn registration(item: &Registration) -> Value {
    json!({
        "schemaVersion":item.schema_version,
        "id":item.id,
        "executable":item.command.first(),
        "argvLength":item.command.len(),
        "workingDirectory":item.working_directory,
        "intervalSeconds":item.interval_seconds,
        "timeoutSeconds":item.timeout_seconds,
        "backendProfile":item.backend_profile,
        "sender":item.sender,
        "target":item.target,
        "attention":item.attention,
        "requiresDisposition":item.requires_disposition,
        "environmentAllowlist":item.environment_allowlist,
    })
}

fn watch(item: &Watch) -> Value {
    let supported = item.schema_version == 2
        && item
            .registration
            .as_ref()
            .is_some_and(|registration| registration.schema_version == 2)
        && item.blocked_reason != Some(BlockedReason::UnsupportedRegistrationVersion);
    let mut value = json!({
        "schemaVersion":item.schema_version,
        "id":item.id,
        "registration":item.registration.as_ref().map(registration),
        "registrationRevision":item.registration_revision,
        "lifecycle":item.lifecycle,
        "eligibility":item.eligibility,
        "health":item.health,
        "blockedReason":item.blocked_reason,
        "activatedAt":item.activated_at,
        "lastSuccessAt":item.last_success_at,
        "lastEventAt":item.last_event_at,
        "lastAttemptId":item.last_attempt_id,
        "runningAttemptId":item.running_attempt_id,
        "runningCancelled":item.running_cancelled,
        "logicalStoreId":item.logical_store_id,
        "observedLogicalStoreId":item.observed_logical_store_id,
        "lastAllocatedEventSequence":item.last_allocated_event_sequence,
        "lastCommittedEventSequence":item.last_committed_event_sequence,
        "pendingOperationId":item.pending_operation_id,
        "nextDueMs":item.next_due_ms,
        "nextAttemptMs":item.next_attempt_ms,
        "suppressedDueCount":item.suppressed_due_count,
        "coalescedDue":item.coalesced_due,
        "consecutiveFailures":item.consecutive_failures,
        "latestAttemptDurationMs":item.latest_attempt_duration_ms,
        "diagnosticCategory":item.diagnostic_category,
        "recurrence":item.recurrence,
        "stateHashStatus":if supported { "canonical-v2" } else { "not-applicable" },
    });
    if supported {
        match hash_value(&item.state) {
            Ok(hash) => value["stateHash"] = json!(hash),
            Err(_) => value["stateHashStatus"] = json!("unavailable"),
        }
    } else {
        value["stateHashReason"] = json!("unsupported-registration-version");
    }
    value
}

fn attempt(item: &Attempt) -> Value {
    json!({
        "schemaVersion":item.schema_version,
        "id":item.id,
        "watchId":item.watch_id,
        "runtimeId":item.runtime_id,
        "registrationRevision":item.registration_revision,
        "startedAt":item.started_at,
        "startedAtMs":item.started_at_ms,
        "finishedAtMs":item.finished_at_ms,
        "executable":item.command.first(),
        "argvLength":item.command.len(),
        "workingDirectory":item.working_directory,
        "priorStateHash":item.prior_state_hash,
        "outcome":item.outcome,
        "diagnosticCategory":item.diagnostic_category,
        "diagnosticPresent":item.diagnostic.is_some(),
        "containmentEvidencePresent":item.containment.is_some(),
        "stateCommitted":item.state_committed,
    })
}

fn operation(item: &Operation) -> Value {
    json!({
        "schemaVersion":item.schema_version,
        "watchId":item.watch_id,
        "eventSequence":item.event_sequence,
        "eventId":item.event_id,
        "operationId":item.operation_id,
        "attemptId":item.attempt_id,
        "attemptNow":item.attempt_now,
        "runtimeId":item.runtime_id,
        "registrationRevision":item.registration_revision,
        "registration":registration(&item.registration),
        "terminal":item.terminal,
        "logicalStoreId":item.recovery.logical_store_id,
        "payloadIdentity":item.recovery.payload_identity,
        "priorStateHash":item.prior_state_hash,
        "proposedStateHash":item.proposed_state_hash,
        "envelopeHash":item.envelope_hash,
        "recurrenceHash":item.recurrence_hash,
        "hashAlgorithm":item.hash_algorithm,
        "hashPreimageVersion":item.hash_preimage_version,
        "status":item.status,
        "queryOnly":item.query_only,
        "reconciliationBudget":item.reconciliation_budget,
        "sendBudget":item.send_budget,
        "nextRecoveryMs":item.next_recovery_ms,
        "lastEvidenceAt":item.last_evidence_at,
        "lastEvidence":item.last_evidence.as_ref().map(send_evidence),
        "receipt":item.receipt.as_ref().map(receipt),
    })
}

fn evidence_state(state: &EvidenceState) -> &'static str {
    match state {
        EvidenceState::Unknown => "unknown",
        EvidenceState::Unavailable => "unavailable",
        EvidenceState::NotAttempted => "not-attempted",
        EvidenceState::Pending => "pending",
        EvidenceState::Accepted => "accepted",
        EvidenceState::Rejected => "rejected",
        EvidenceState::Disposition(_) => "disposition",
        EvidenceState::Quarantined { .. } => "quarantined",
    }
}

fn receipt(item: &SendResult) -> Value {
    json!({
        "logicalStoreId":item.logical_store_id,
        "operationId":item.operation_id,
        "messageId":item.message_id,
        "threadId":item.thread_id,
        "sender":item.sender,
        "recipient":item.recipient,
        "payloadIdentity":item.payload_identity,
        "replayed":item.replayed,
        "axes":{
            "durableAcceptance":evidence_state(&item.axes.durable_acceptance),
            "occupiedAtAcceptance":item.axes.occupied_at_acceptance,
            "pushAcceptance":evidence_state(&item.axes.push_acceptance),
            "recipientConsumption":evidence_state(&item.axes.recipient_consumption),
            "workflowDisposition":evidence_state(&item.axes.workflow_disposition),
        },
    })
}

fn send_evidence(item: &SendEvidence) -> Value {
    match item {
        SendEvidence::ObservationCancelled => json!({"source":"observation-cancelled"}),
        SendEvidence::Send(Ok(item)) => json!({"source":"send","receipt":receipt(item)}),
        SendEvidence::Send(Err(error)) => {
            json!({"source":"send","error":client_error(error)})
        }
        SendEvidence::Reconciliation(Err(error)) => {
            json!({"source":"reconciliation","error":client_error(error)})
        }
        SendEvidence::Reconciliation(Ok(item)) => {
            let result = match item {
                OperationReconciliation::Recorded(item) => {
                    let outcome = match &item.outcome {
                        RecordedOperationOutcome::Accepted(item) => {
                            json!({"status":"accepted","receipt":receipt(item)})
                        }
                        RecordedOperationOutcome::Duplicate(item) => {
                            json!({"status":"duplicate","receipt":receipt(item)})
                        }
                        RecordedOperationOutcome::Rejected(error) => {
                            json!({"status":"rejected","error":client_error(error)})
                        }
                        RecordedOperationOutcome::Partial { error, .. } => {
                            json!({"status":"partial","error":error.as_ref().map(client_error)})
                        }
                        RecordedOperationOutcome::Indeterminate { error, .. } => {
                            json!({"status":"indeterminate","error":error.as_ref().map(client_error)})
                        }
                        RecordedOperationOutcome::Pending { .. } => json!({"status":"pending"}),
                    };
                    json!({
                        "status":"recorded",
                        "logicalStoreId":item.logical_store_id,
                        "operationId":item.operation_id,
                        "sender":item.sender,
                        "payloadIdentity":item.payload_identity,
                        "retryBudget":item.retry_budget,
                        "outcome":outcome,
                    })
                }
                OperationReconciliation::NotRecorded(item) => json!({
                    "status":"not-recorded",
                    "logicalStoreId":item.logical_store_id,
                    "operationId":item.operation_id,
                    "payloadIdentity":item.payload_identity,
                    "retentionGeneration":item.retention_generation,
                }),
                OperationReconciliation::RetentionBoundaryCrossed {
                    staged_generation,
                    current_generation,
                } => json!({
                    "status":"retention-boundary-crossed",
                    "stagedGeneration":staged_generation,
                    "currentGeneration":current_generation,
                }),
            };
            json!({"source":"reconciliation","result":result})
        }
    }
}

fn membership_reason(reason: &MembershipLossReason) -> &'static str {
    match reason {
        MembershipLossReason::DaemonRestart => "daemon-restart",
        MembershipLossReason::PredicateDeath => "predicate-death",
        MembershipLossReason::Collision => "collision",
        MembershipLossReason::DeliberateDetach => "deliberate-detach",
        MembershipLossReason::NeedsAttach => "needs-attach",
        MembershipLossReason::OwnerDemoted => "owner-demoted",
        MembershipLossReason::Unknown { .. } => "unknown",
    }
}

fn client_error(error: &ApplicationClientError) -> Value {
    let code = match error {
        ApplicationClientError::MembershipLost {
            address, reason, ..
        } => {
            return json!({"code":"membership-lost","address":address,"reason":membership_reason(reason)});
        }
        ApplicationClientError::RejectedBeforeAcceptance { retryability, .. } => {
            return json!({"code":"rejected-before-acceptance","retryability":retryability});
        }
        ApplicationClientError::Collision(item) => {
            return json!({
                "code":"collision",
                "address":item.address,
                "ownerInstanceId":item.owner_instance_id,
                "leaseEpoch":item.lease_epoch,
            });
        }
        ApplicationClientError::AmbiguousSender(_) => "ambiguous-sender",
        ApplicationClientError::UnsupportedCapability(_) => "unsupported-capability",
        ApplicationClientError::DeliveryMismatch { .. } => "delivery-mismatch",
        ApplicationClientError::OperationMismatch {
            operation_id,
            evidence,
        } => {
            return json!({
                "code":"operation-mismatch",
                "operationId":operation_id,
                "attempted":evidence.attempted,
                "existing":evidence.existing,
            });
        }
        ApplicationClientError::StoreBindingMismatch { staged, current } => {
            return json!({"code":"store-binding-mismatch","staged":staged,"current":current});
        }
        ApplicationClientError::ResyncRequired { .. } => "resync-required",
        ApplicationClientError::Partial(_) => "partial",
        ApplicationClientError::Indeterminate { .. } => "indeterminate",
        ApplicationClientError::InvalidRequest(_) => "invalid-request",
        ApplicationClientError::Protocol { .. } => "protocol",
        ApplicationClientError::DeliveryQuarantined { .. } => "delivery-quarantined",
        ApplicationClientError::TransportUncertain(_) => "transport-uncertain",
        ApplicationClientError::Unavailable(_) => "unavailable",
        ApplicationClientError::DaemonBootstrap(reason) => {
            return json!({"code":"daemon-bootstrap","reason":reason});
        }
        _ => "application-client-error",
    };
    json!({"code":code})
}

fn membership(item: &ApplicationHealth) -> Value {
    json!({
        "logicalStoreId":item.logical_store_id,
        "runtimeId":item.runtime_id,
        "address":item.address,
        "capability":item.capability,
        "registered":item.registered,
        "leaseEpoch":item.lease_epoch,
        "ownerInstanceId":item.owner_instance_id,
        "pendingUnconsumed":item.pending_unconsumed,
        "inboundActionable":item.inbound_actionable,
        "acknowledgmentPending":item.acknowledgment_pending,
        "outstandingAckCount":item.outstanding_ack_count,
        "senderReady":item.sender_ready,
        "receiveReady":item.receive_ready,
        "attendedButDeaf":item.attended_but_deaf,
        "recovering":item.recovering,
        "lastRecoveryFailurePresent":item.last_recovery_failure.is_some(),
        "lifecycle":item.lifecycle.iter().map(membership_lifecycle).collect::<Vec<_>>(),
        "liveness":item.liveness.iter().map(|process| json!({
            "pid":process.pid,
            "startTime":process.start_time,
            "alive":process.alive,
        })).collect::<Vec<_>>(),
        "principalVerification":item.principal.verification,
        "degraded":item.degraded,
        "stoppedOrUnattended":item.stopped_or_unattended,
    })
}

fn membership_lifecycle(item: &ApplicationLifecycleEvidence) -> Value {
    match item {
        ApplicationLifecycleEvidence::MembershipLoss { reason, .. } => {
            json!({"type":"membership-loss","reason":membership_reason(reason)})
        }
        ApplicationLifecycleEvidence::Collision(item) => json!({
            "type":"collision",
            "address":item.address,
            "ownerInstanceId":item.owner_instance_id,
            "leaseEpoch":item.lease_epoch,
        }),
        ApplicationLifecycleEvidence::CompensationPending(_) => {
            json!({"type":"compensation-pending"})
        }
        ApplicationLifecycleEvidence::DeliberateDetach {
            runtime_id, at_ms, ..
        } => json!({"type":"deliberate-detach","runtimeId":runtime_id,"atMs":at_ms}),
        ApplicationLifecycleEvidence::Reconciliation { state, .. } => {
            json!({"type":"reconciliation","state":state})
        }
    }
}

fn lifecycle(item: &MultiAddressOutcome) -> Value {
    let results: serde_json::Map<_, _> = item
        .results
        .iter()
        .map(|(address, result)| {
            let value = match result {
                AddressLifecycleResult::Attached(_) => json!({"status":"attached"}),
                AddressLifecycleResult::Reconciled(_) => json!({"status":"reconciled"}),
                AddressLifecycleResult::Detached(_) => json!({"status":"detached"}),
                AddressLifecycleResult::Failed(error) => {
                    json!({"status":"failed","error":client_error(error)})
                }
            };
            (address.clone(), value)
        })
        .collect();
    json!({
        "ready":item.ready,
        "results":results,
        "compensationCount":item.compensation.len(),
        "validationError":item.validation_error.as_ref().map(client_error),
        "cancellation":item.cancellation,
    })
}

fn runtime_health(item: &RuntimeHealth, now_ms: i64) -> Value {
    let heartbeat_age_ms = chrono::DateTime::parse_from_rfc3339(&item.heartbeat_at)
        .ok()
        .map(|heartbeat| now_ms.saturating_sub(heartbeat.timestamp_millis()).max(0));
    let stale = heartbeat_age_ms.is_none_or(|age| age > HEARTBEAT_STALE_AFTER_MS);
    json!({
        "schemaVersion":item.schema_version,
        "runtimeId":item.runtime_id,
        "pid":item.pid,
        "startedAt":item.started_at,
        "heartbeatAt":item.heartbeat_at,
        "heartbeatAgeMs":heartbeat_age_ms,
        "heartbeatStaleAfterMs":HEARTBEAT_STALE_AFTER_MS,
        "heartbeatStale":stale,
        "status":item.status,
        "productionReady":item.production_ready,
        "registryRevision":item.registry_revision,
        "reconciledRegistryRevision":item.reconciled_registry_revision,
        "restartReconciled":item.restart_reconciled,
        "senders":item.senders.iter().map(|sender| json!({
            "backendProfile":sender.backend_profile,
            "requiredSenders":sender.required_senders,
            "logicalStoreId":sender.logical_store_id,
            "ready":sender.ready,
            "membership":sender.membership.iter().map(membership).collect::<Vec<_>>(),
            "lifecycle":sender.lifecycle.as_ref().map(lifecycle),
            "compensation":sender.compensation.iter().map(lifecycle).collect::<Vec<_>>(),
            "error":sender.error.as_ref().map(client_error),
        })).collect::<Vec<_>>(),
        "retention":item.retention,
        "diagnostics":item.diagnostics,
        "detach":item.detach.iter().map(lifecycle).collect::<Vec<_>>(),
    })
}

pub fn transport_error_message(code: &str) -> &'static str {
    match code {
        "runtime-not-running" => "no owning runtime is listening; start telex-watcher run first",
        "timeout" => "local management timed out; inspect state before retrying a mutation",
        "outcome-unknown" => "the admitted outcome is unknown; inspect state before retrying",
        "unauthorized" => "local management requires the runtime's OS user",
        "invalid-request" => "the runtime rejected the local management request",
        "busy" => "the local management admission limit is busy; retry later",
        "response-too-large" => "the response exceeds the transport limit; narrow the query",
        "runtime-error" => "the owner could not complete the request; inspect runtime health",
        _ => "local management transport failed",
    }
}

fn refusal_message(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::InvalidRegistration => "registration does not satisfy the v2 policy",
        Refusal::InvalidState => "replacement state does not satisfy strict v2 JSON limits",
        Refusal::NoPendingOperation => "the watch has no pending operation",
        Refusal::RecoveryUnavailable => "exact recovery evidence is unavailable",
        Refusal::NotFound => "the watch ID does not exist",
        Refusal::IdAlreadyAllocated => "the permanent watch ID is already allocated",
        Refusal::PendingOperation => "a pending operation must be reconciled or removed first",
        Refusal::InvalidLifecycle => "the operation is not allowed in the current lifecycle",
        Refusal::ImmutableRegistration => "ID, backend, sender and target cannot be changed",
        Refusal::UnsupportedRegistrationVersion => {
            "the record is unsupported v1; only removal is allowed"
        }
        Refusal::BlockedReasonUncorrected => {
            "the blocked condition must be corrected before resuming"
        }
        Refusal::InvalidGrant => "a finite grant requires 1..100 attempts and 1..86400 seconds",
        Refusal::WrongRecoveryProof => {
            "the exact operation lacks authoritative proof for this recovery"
        }
        Refusal::AttemptChanged => "the attempt changed before the operation could be applied",
    }
}
