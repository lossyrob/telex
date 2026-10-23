//! Acceptance is a separate axis from delivery, attendance, and disposition.

use crate::evidence::{BlockedReason, Operation, SendEvidence};
use telex::application_client::{
    ApplicationClientError, EvidenceState, OperationReconciliation, OperationTarget,
    PayloadIdentity, RecordedOperationOutcome, RecoveryHandle, RejectionRetryability, SendResult,
};

#[derive(Debug)]
pub(crate) enum Resolution {
    Accepted(Box<SendResult>),
    Retry,
    PermanentRejection,
    Indeterminate,
    Conflict(BlockedReason),
}

pub(crate) fn classify(operation: &Operation, evidence: &SendEvidence) -> Resolution {
    match evidence {
        SendEvidence::Send(Ok(receipt)) => receipt_resolution(operation, receipt),
        SendEvidence::Send(Err(error)) | SendEvidence::Reconciliation(Err(error)) => {
            error_resolution(operation, error)
        }
        SendEvidence::ObservationCancelled => Resolution::Indeterminate,
        SendEvidence::Reconciliation(Ok(OperationReconciliation::RetentionBoundaryCrossed {
            ..
        })) => Resolution::Indeterminate,
        SendEvidence::Reconciliation(Ok(OperationReconciliation::NotRecorded(proof))) => {
            if proof.logical_store_id != operation.recovery.logical_store_id {
                return Resolution::Conflict(BlockedReason::LogicalStoreMismatch);
            }
            if proof.operation_id != operation.recovery.operation_id
                || proof.responsibility != operation.recovery.responsibility
                || !same_payload(
                    &proof.payload_identity,
                    &operation.recovery.payload_identity,
                )
                || Some(proof.retention_generation) != operation.recovery.retention_generation
            {
                return Resolution::Conflict(BlockedReason::OperationIdentityConflict);
            }
            Resolution::Retry
        }
        SendEvidence::Reconciliation(Ok(OperationReconciliation::Recorded(record))) => {
            if record.logical_store_id != operation.recovery.logical_store_id {
                return Resolution::Conflict(BlockedReason::LogicalStoreMismatch);
            }
            if record.operation_id != operation.recovery.operation_id
                || record.responsibility != operation.recovery.responsibility
                || !same_payload(
                    &record.payload_identity,
                    &operation.recovery.payload_identity,
                )
                || record.sender != operation.envelope.sender
                || record.target
                    != (OperationTarget::Send {
                        to: operation.envelope.to.clone(),
                        cc: operation.envelope.cc.clone(),
                    })
                || record.retry_budget != operation.envelope.retry_budget
            {
                return Resolution::Conflict(BlockedReason::OperationIdentityConflict);
            }
            match &record.outcome {
                RecordedOperationOutcome::Accepted(receipt)
                | RecordedOperationOutcome::Duplicate(receipt) => {
                    receipt_resolution(operation, receipt)
                }
                RecordedOperationOutcome::Rejected(error) => error_resolution(operation, error),
                RecordedOperationOutcome::Partial { error, recovery } => {
                    if let Some(handle) = recovery {
                        if let Some(reason) = handle_conflict(operation, handle) {
                            return Resolution::Conflict(reason);
                        }
                    }
                    error
                        .as_ref()
                        .map(|error| error_resolution(operation, error))
                        .unwrap_or(Resolution::Indeterminate)
                }
                RecordedOperationOutcome::Pending { recovery }
                | RecordedOperationOutcome::Indeterminate { recovery, .. } => {
                    handle_conflict(operation, recovery)
                        .map(Resolution::Conflict)
                        .unwrap_or(Resolution::Indeterminate)
                }
            }
        }
    }
}

fn same_payload(left: &PayloadIdentity, right: &PayloadIdentity) -> bool {
    left.comparable
        && right.comparable
        && left.algorithm == right.algorithm
        && left.digest == right.digest
}

fn handle_conflict(operation: &Operation, handle: &RecoveryHandle) -> Option<BlockedReason> {
    if handle.logical_store_id != operation.recovery.logical_store_id {
        Some(BlockedReason::LogicalStoreMismatch)
    } else if handle.responsibility != operation.recovery.responsibility
        || handle.operation_id != operation.recovery.operation_id
        || !same_payload(
            &handle.payload_identity,
            &operation.recovery.payload_identity,
        )
    {
        Some(BlockedReason::OperationIdentityConflict)
    } else {
        None
    }
}

fn receipt_resolution(operation: &Operation, receipt: &SendResult) -> Resolution {
    if receipt.logical_store_id != operation.recovery.logical_store_id {
        return Resolution::Conflict(BlockedReason::LogicalStoreMismatch);
    }
    if receipt.operation_id != operation.recovery.operation_id
        || !same_payload(
            &receipt.payload_identity,
            &operation.recovery.payload_identity,
        )
        || receipt.sender != operation.envelope.sender
        || receipt.recipient != operation.envelope.to
        || receipt.message_id <= 0
        || receipt.thread_id <= 0
    {
        return Resolution::Conflict(BlockedReason::OperationIdentityConflict);
    }
    if receipt.axes.durable_acceptance == EvidenceState::Accepted {
        Resolution::Accepted(Box::new(receipt.clone()))
    } else {
        Resolution::Indeterminate
    }
}

fn error_resolution(operation: &Operation, error: &ApplicationClientError) -> Resolution {
    match error {
        ApplicationClientError::RejectedBeforeAcceptance { retryability, .. } => match retryability
        {
            RejectionRetryability::Transient => Resolution::Retry,
            RejectionRetryability::Permanent => Resolution::PermanentRejection,
        },
        ApplicationClientError::OperationMismatch { .. } => {
            Resolution::Conflict(BlockedReason::OperationIdentityConflict)
        }
        ApplicationClientError::StoreBindingMismatch { .. } => {
            Resolution::Conflict(BlockedReason::LogicalStoreMismatch)
        }
        ApplicationClientError::Indeterminate { recovery, .. } => {
            handle_conflict(operation, recovery)
                .map(Resolution::Conflict)
                .unwrap_or(Resolution::Indeterminate)
        }
        // Membership loss, collisions, unavailable history, transport failure,
        // and future client variants never prove absence of acceptance.
        _ => Resolution::Indeterminate,
    }
}
