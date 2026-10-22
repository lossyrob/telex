use crate::client::bounded;
use crate::evidence::{
    Attempt, Budget, EventMetadata, EventProposal, Operation, OperationStatus, Watch,
    WatcherProvenance,
};
use crate::protocol_v2::{canonical_bytes, hash_value};
use anyhow::{Context, Result};
use serde_json::json;
use telex::application_client::{ApplicationClient, OperationId, SendRequest};
use uuid::Uuid;

pub(crate) async fn prepare(
    client: &ApplicationClient,
    watch: &Watch,
    attempt: &Attempt,
    proposal: EventProposal,
    now: i64,
) -> Result<Operation> {
    let registration = watch
        .registration
        .as_ref()
        .context("unsupported registration")?
        .clone();
    let event_sequence = watch
        .last_allocated_event_sequence
        .checked_add(1)
        .filter(|sequence| *sequence <= 9_007_199_254_740_991)
        .context("event-sequence-exhausted")?;
    let event_id = format!("watcher-event-{}", Uuid::new_v4());
    let operation_id = format!("watcher-operation-{}", Uuid::new_v4());
    let detector = serde_json::to_value(&proposal.event)?
        .get("metadata")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    let metadata = EventMetadata {
        schema_version: 2,
        watcher: WatcherProvenance {
            watch_id: watch.id.clone(),
            event_sequence,
            event_id: event_id.clone(),
            operation_id: operation_id.clone(),
            attempt_id: attempt.id.clone(),
            runtime_id: attempt.runtime_id.clone(),
            logical_store_id: client.logical_store_id().0.clone(),
            registration_revision: watch.registration_revision,
            detector_schema_version: 2,
        },
        detector: detector.clone(),
    };
    let metadata_bytes = canonical_bytes(&metadata)?;
    anyhow::ensure!(
        metadata_bytes.len() <= 80 * 1024,
        "normalized metadata exceeds 80 KiB"
    );
    let attention = serde_json::to_value(&registration.attention)?
        .as_str()
        .context("attention must serialize as a string")?
        .to_owned();
    let envelope = SendRequest {
        operation_id: OperationId(operation_id.clone()),
        sender: registration.sender.clone(),
        to: registration.target.clone(),
        cc: Vec::new(),
        kind: proposal.event.kind.clone(),
        attention: attention.clone(),
        requires_disposition: registration.requires_disposition,
        subject: Some(proposal.event.subject.clone()),
        body: proposal.event.body.clone(),
        metadata: Some(String::from_utf8(metadata_bytes)?),
        retry_budget: 3,
    };
    let recovery = bounded(client.prepare_send(&envelope)).await?;
    let recurrence_hash = hash_value(&json!({
        "schemaVersion": 1,
        "route": {
            "backendProfile": registration.backend_profile, "sender": registration.sender,
            "target": registration.target, "attention": attention,
            "requiresDisposition": registration.requires_disposition,
        },
        "event": {
            "kind": proposal.event.kind, "subject": proposal.event.subject,
            "body": proposal.event.body, "metadata": detector,
        }
    }))?;
    Ok(Operation {
        schema_version: 2,
        watch_id: watch.id.clone(),
        event_sequence,
        event_id,
        operation_id,
        attempt_id: attempt.id.clone(),
        attempt_now: attempt.started_at.clone(),
        runtime_id: attempt.runtime_id.clone(),
        registration_revision: watch.registration_revision,
        registration,
        prior_state: watch.state.clone(),
        prior_state_hash: hash_value(&watch.state)?,
        proposed_state_hash: hash_value(&proposal.next_state)?,
        proposed_state: proposal.next_state,
        terminal: proposal.terminal,
        envelope_hash: hash_value(&envelope)?,
        envelope,
        recovery,
        recurrence_hash,
        hash_algorithm: "sha256".into(),
        hash_preimage_version: 1,
        status: OperationStatus::Staged,
        query_only: false,
        reconciliation_budget: Budget {
            remaining: 8,
            used: 0,
            deadline_ms: now.saturating_add(300_000),
        },
        send_budget: Budget {
            remaining: 3,
            used: 0,
            deadline_ms: now.saturating_add(300_000),
        },
        next_recovery_ms: now,
        last_evidence_at: None,
        last_evidence: None,
        receipt: None,
    })
}

pub(crate) fn verify(operation: &Operation) -> Result<()> {
    anyhow::ensure!(
        operation.schema_version == 2
            && operation.hash_algorithm == "sha256"
            && operation.hash_preimage_version == 1,
        "unsupported operation evidence version"
    );
    anyhow::ensure!(
        operation.envelope_hash == hash_value(&operation.envelope)?
            && operation.prior_state_hash == hash_value(&operation.prior_state)?
            && operation.proposed_state_hash == hash_value(&operation.proposed_state)?,
        "frozen operation hash mismatch"
    );
    let metadata: EventMetadata = serde_json::from_str(
        operation
            .envelope
            .metadata
            .as_deref()
            .context("operation metadata missing")?,
    )?;
    anyhow::ensure!(
        metadata.schema_version == 2
            && metadata.watcher.watch_id == operation.watch_id
            && metadata.watcher.event_sequence == operation.event_sequence
            && metadata.watcher.event_id == operation.event_id
            && metadata.watcher.operation_id == operation.operation_id
            && metadata.watcher.attempt_id == operation.attempt_id
            && metadata.watcher.runtime_id == operation.runtime_id
            && metadata.watcher.logical_store_id == operation.recovery.logical_store_id.0
            && metadata.watcher.registration_revision == operation.registration_revision
            && metadata.watcher.detector_schema_version == 2
            && operation.recovery.operation_id.0 == operation.operation_id
            && operation.envelope.operation_id.0 == operation.operation_id
            && operation.envelope.sender == operation.registration.sender
            && operation.envelope.to == operation.registration.target
            && operation.envelope.cc.is_empty()
            && operation.envelope.requires_disposition
                == operation.registration.requires_disposition
            && serde_json::to_value(&operation.registration.attention)?
                == operation.envelope.attention,
        "frozen operation identity mismatch"
    );
    Ok(())
}
