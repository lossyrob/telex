use telex::application_client::{
    AckResult, AddressSpec, ApplicationClient, ApplicationClientConfig, ApplicationClientError,
    ApplicationHealth, ApplicationResponsibility, ApplicationStoreMaintenance, DeltaPage,
    ExactDeliveryIdentity, HistoryItem, LifecycleCancellationEvidence, LifecycleOperationKind,
    LogicalStoreId, MultiAddressOutcome, OperationId, OperationReconciliation,
    PrincipalProvenance, ReceivedDelivery, RecoveryHandle, RecoveryPolicy, ReplyRequest,
    SendRequest, SendResult, SourceReference, SourceResolution,
};
use telex::model::{
    ApplicationStorageStats, CleanupReport, CompoundStepRecord, CompoundStepState, DeliveryRow,
    DispositionRow, MessageRow, RetentionPolicy, StateDeltaRecord, StoreDeltaCleanupReport,
    StoreDeltaRetentionPolicy,
};

pub async fn connect(config: ApplicationClientConfig) -> Result<ApplicationClient, ApplicationClientError> {
    ApplicationClient::connect(config).await
}

pub async fn cancel_attach_after_one_step(
    client: &ApplicationClient,
    specs: &[AddressSpec],
) -> MultiAddressOutcome {
    let mut operation = client.begin_attach(specs);
    operation.advance().await;
    operation.cancelled_outcome()
}

pub async fn attach_until_cancelled(
    client: &ApplicationClient,
    specs: &[AddressSpec],
    cancelled: tokio::sync::oneshot::Receiver<()>,
) -> MultiAddressOutcome {
    let mut operation = client.begin_attach(specs);
    tokio::select! {
        outcome = operation.run() => outcome,
        _ = cancelled => operation.cancelled_outcome(),
    }
}

pub async fn prepare_and_send(
    client: &ApplicationClient,
    request: SendRequest,
) -> Result<(RecoveryHandle, SendResult), ApplicationClientError> {
    let recovery = client.prepare_send(&request).await?;
    let result = client.send(request).await?;
    Ok((recovery, result))
}

pub async fn prepare_reply(
    client: &ApplicationClient,
    request: &ReplyRequest,
) -> Result<RecoveryHandle, ApplicationClientError> {
    client.prepare_reply(request).await
}

pub async fn reconcile(
    client: &ApplicationClient,
    recovery: &RecoveryHandle,
) -> Result<OperationReconciliation, ApplicationClientError> {
    client.reconcile_operation(recovery).await
}

pub async fn acknowledge(
    client: &ApplicationClient,
    delivery: &ReceivedDelivery,
) -> Result<AckResult, ApplicationClientError> {
    client.acknowledge(&delivery.ack).await
}

#[allow(clippy::type_complexity)]
pub fn supported_contract_types(
    _: (
        ApplicationResponsibility,
        LogicalStoreId,
        OperationId,
        ExactDeliveryIdentity,
        LifecycleOperationKind,
        LifecycleCancellationEvidence,
        RecoveryPolicy,
        ApplicationHealth,
        PrincipalProvenance,
        SourceReference,
        SourceResolution,
        HistoryItem,
        DeltaPage,
        MessageRow,
        DeliveryRow,
        DispositionRow,
        StateDeltaRecord,
        CompoundStepRecord,
        CompoundStepState,
        RetentionPolicy,
        CleanupReport,
        StoreDeltaRetentionPolicy,
        StoreDeltaCleanupReport,
        ApplicationStorageStats,
        ApplicationStoreMaintenance,
    ),
) {
}
