//! Executable, consumer-shaped probes for the supported Application Client.
//!
//! This crate is an *external consumer* stand-in. It is deliberately built with
//! `telex` default features disabled and uses **only** the public
//! `telex::application_client` surface plus the stable `telex::model` types
//! that surface returns or accepts.
//!
//! It never reaches for daemon, install, backend, IPC, CLI, product DTO,
//! detector/Station policy, embedded-serve, or sidecar code. Two shapes are
//! provided:
//!
//! * [`run_watcher_probe`] -- a send-only Watcher: it emits and must never gain
//!   inbound attendance.
//! * [`run_operator_station_probe`] -- a bidirectional Operator Station: it
//!   attends, replies, dispositions, reconciles, and maintains its store.
//!
//! Both connect through the production seam
//! `ApplicationDaemonBootstrap::InstalledCurrent { trusted_root }`.

use std::path::PathBuf;

use telex::application_client::{
    AckResult, AddressSpec, ApplicationCapability, ApplicationClient, ApplicationClientConfig,
    ApplicationClientError, ApplicationDaemonBootstrap, ApplicationHealth,
    ApplicationLifecycleEvidence, ApplicationResponsibility, ApplicationStoreMaintenance,
    CompensationAction, CompoundDispositionRequest, CompoundStep, DeltaPage, EvidenceState,
    ExactDeliveryIdentity, HistoryItem, LifecycleCancellationEvidence, LifecycleOperationKind,
    LogicalStoreId, MultiAddressOutcome, OperationId, OperationReconciliation, PrincipalProvenance,
    PrincipalVerification, ReceivedDelivery, RecoveryHandle, RecoveryPolicy, ReplyRequest,
    SendRequest, SendResult, SourceReference, SourceResolution,
};
use telex::model::{
    ApplicationStorageStats, CleanupReport, CompoundStepRecord, CompoundStepState, DeliveryRow,
    DispositionRow, MessageRow, RetentionPolicy, StateDeltaRecord, StoreDeltaCleanupReport,
    StoreDeltaRetentionPolicy,
};

/// How one probe run selects its store, its trusted install root, and its
/// address namespace. Every field is supplied by the caller (library API) or by
/// the environment (binary); the fixture never parses a Telex CLI.
#[derive(Clone, Debug)]
pub struct ProbeConfig {
    /// Explicit backend profile name. The supported consumer shape always names
    /// its backend rather than relying on an ambient default.
    pub backend: Option<String>,
    /// Absolute, explicitly trusted install root for `InstalledCurrent`.
    pub trusted_root: PathBuf,
    /// Prefix that keeps one probe run's addresses distinct.
    pub run_id: String,
}

impl ProbeConfig {
    /// Read a probe configuration from the environment.
    pub fn from_env() -> Result<Self, String> {
        let trusted_root = std::env::var("TELEX_FIXTURE_TRUSTED_ROOT")
            .map_err(|_| "TELEX_FIXTURE_TRUSTED_ROOT is required".to_string())?;
        Ok(Self {
            backend: std::env::var("TELEX_FIXTURE_BACKEND")
                .ok()
                .filter(|value| !value.is_empty()),
            trusted_root: PathBuf::from(trusted_root),
            run_id: std::env::var("TELEX_FIXTURE_RUN_ID").unwrap_or_else(|_| "run".to_string()),
        })
    }

    fn address(&self, suffix: &str) -> String {
        format!("fixture:{}:{suffix}", self.run_id)
    }

    async fn connect(&self, responsibility: &str) -> Result<ApplicationClient, String> {
        ApplicationClient::connect_with_daemon(
            ApplicationClientConfig {
                responsibility: ApplicationResponsibility(responsibility.to_string()),
                backend: self.backend.clone(),
                db_override: None,
            },
            ApplicationDaemonBootstrap::InstalledCurrent {
                trusted_root: self.trusted_root.clone(),
            },
        )
        .await
        .map_err(|e| format!("connect {responsibility}: {e}"))
    }
}

/// Evidence a probe collected, in the order it was observed.
#[derive(Default, Debug)]
pub struct ProbeReport {
    pub lines: Vec<String>,
}

impl ProbeReport {
    fn record(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }
}

fn spec(address: &str, capability: ApplicationCapability, description: &str) -> AddressSpec {
    AddressSpec {
        address: address.to_string(),
        capability,
        description: Some(description.to_string()),
        scope: None,
        tags: None,
    }
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_string())
    }
}

// ----------------------------------------------------------------------------------------
// Watcher-shaped send-only probe
// ----------------------------------------------------------------------------------------

/// A long-lived Watcher: it emits observations and must never look attended.
pub async fn run_watcher_probe(config: &ProbeConfig) -> Result<ProbeReport, String> {
    let mut report = ProbeReport::default();

    // Production bootstrap through the explicitly trusted install root.
    let client = config.connect("fixture-watcher").await?;
    let store_id: LogicalStoreId = client.logical_store_id().clone();
    let runtime = client.runtime_id().clone();
    report.record(format!("responsibility={}", client.responsibility().0));
    report.record(format!("store={}", store_id.0));

    // A second connection is a distinct runtime over the same logical store.
    let reconnected = config.connect("fixture-watcher").await?;
    require(
        reconnected.logical_store_id() == &store_id,
        "logical store identity must be stable across reconnect",
    )?;
    require(
        reconnected.runtime_id() != &runtime,
        "each connection must mint a fresh runtime identity",
    )?;
    report.record("identity=stable-store-fresh-runtime");

    let watcher = spec(
        &config.address("watcher"),
        ApplicationCapability::SendOnly,
        "watcher send-only probe",
    );
    let station = spec(
        &config.address("watcher-target"),
        ApplicationCapability::Bidirectional,
        "watcher target",
    );
    let attach: MultiAddressOutcome = client.attach(std::slice::from_ref(&watcher)).await;
    require(attach.ready, "watcher attach must be ready")?;
    report.record(format!("attached={}", watcher.address));

    // The target is attended by a separate responsibility, so occupancy at
    // acceptance is a real observation rather than an artifact.
    let target_client = config.connect("fixture-watcher-target").await?;
    let target_attach = target_client.attach(std::slice::from_ref(&station)).await;
    require(target_attach.ready, "target attach must be ready")?;

    // Prepare the exact-operation recovery handle *before* the first attempt.
    let request = SendRequest {
        operation_id: OperationId(format!("watcher-{}", config.run_id)),
        sender: watcher.address.clone(),
        to: station.address.clone(),
        cc: Vec::new(),
        kind: "note".to_string(),
        attention: "background".to_string(),
        requires_disposition: true,
        subject: Some("watcher observation".to_string()),
        body: "observed something worth reporting".to_string(),
        metadata: Some(r#"{"source":"watcher-fixture"}"#.to_string()),
        retry_budget: 2,
    };
    let recovery: RecoveryHandle = client
        .prepare_send(&request)
        .await
        .map_err(|e| format!("prepare_send: {e}"))?;
    require(
        recovery.payload_identity.comparable && recovery.retention_generation.is_some(),
        "a prepared handle must carry a comparable payload identity and its retention generation",
    )?;

    let sent: SendResult = client
        .send(request.clone())
        .await
        .map_err(|e| format!("send: {e}"))?;
    require(
        sent.axes.durable_acceptance == EvidenceState::Accepted,
        "durable acceptance must be decided by the send",
    )?;
    require(
        sent.axes.occupied_at_acceptance.is_some(),
        "occupancy at acceptance is reported as its own axis",
    )?;
    require(
        sent.axes.push_acceptance == EvidenceState::Unknown
            && sent.axes.recipient_consumption == EvidenceState::Unknown
            && sent.axes.workflow_disposition == EvidenceState::Unknown,
        "acceptance must not imply push, consumption, or disposition",
    )?;
    report.record(format!(
        "sent message={} durable={:?} occupied={:?}",
        sent.message_id, sent.axes.durable_acceptance, sent.axes.occupied_at_acceptance
    ));

    // A retry of the same operation replays instead of duplicating.
    let replay = client
        .send(request.clone())
        .await
        .map_err(|e| format!("replay send: {e}"))?;
    require(
        replay.replayed && replay.message_id == sent.message_id,
        "a retried operation must replay its exact result",
    )?;
    report.record("retry=replayed");

    // Exact-operation recovery from the prepared handle.
    match client
        .reconcile_operation(&recovery)
        .await
        .map_err(|e| format!("reconcile_operation: {e}"))?
    {
        OperationReconciliation::Recorded(record) => {
            require(
                record.operation_id == request.operation_id,
                "recovery must resolve the exact operation",
            )?;
            report.record("recovery=recorded");
        }
        other => return Err(format!("expected a recorded operation, got {other:?}")),
    }

    // A send-only membership must never gain inbound attendance.
    match client.receive(&watcher.address, Some(50)).await {
        Err(ApplicationClientError::UnsupportedCapability(_)) => {}
        other => return Err(format!("send-only receive must be refused, got {other:?}")),
    }
    match client
        .history(Some(watcher.address.clone()), false, None, None, None, 10)
        .await
    {
        Err(ApplicationClientError::UnsupportedCapability(_)) => {}
        other => return Err(format!("send-only history must be refused, got {other:?}")),
    }
    let health: Vec<ApplicationHealth> =
        client.health().await.map_err(|e| format!("health: {e}"))?;
    let watcher_health = health
        .iter()
        .find(|record| record.address == watcher.address)
        .ok_or_else(|| "watcher health record is missing".to_string())?;
    require(
        watcher_health.sender_ready
            && !watcher_health.receive_ready
            && !watcher_health.attended_but_deaf
            && watcher_health.pending_unconsumed == 0
            && watcher_health.inbound_actionable == 0,
        "a send-only Watcher must never present as inbound-attended",
    )?;
    report.record("attendance=send-only");

    // Retention-boundary handling: a handle staged before a prune can no longer
    // claim authoritative absence, and a fresh handle can.
    let unsent = SendRequest {
        operation_id: OperationId(format!("watcher-unsent-{}", config.run_id)),
        body: "never attempted".to_string(),
        ..request.clone()
    };
    let staged = client
        .prepare_send(&unsent)
        .await
        .map_err(|e| format!("prepare unsent: {e}"))?;
    match client
        .reconcile_operation(&staged)
        .await
        .map_err(|e| format!("reconcile unsent: {e}"))?
    {
        OperationReconciliation::NotRecorded(evidence) => require(
            Some(evidence.retention_generation) == staged.retention_generation,
            "NotRecorded must be exact within its retention generation",
        )?,
        other => return Err(format!("expected NotRecorded, got {other:?}")),
    }
    let cleanup: CleanupReport = client
        .cleanup(RetentionPolicy {
            completed_before_ms: i64::MAX,
            max_delete: 100,
        })
        .await
        .map_err(|e| format!("cleanup: {e}"))?;
    require(
        cleanup.operations_deleted >= 1,
        "cleanup must prune the completed operation",
    )?;
    match client
        .reconcile_operation(&staged)
        .await
        .map_err(|e| format!("reconcile across retention: {e}"))?
    {
        OperationReconciliation::RetentionBoundaryCrossed {
            staged_generation,
            current_generation,
        } => {
            require(
                staged_generation == staged.retention_generation
                    && Some(current_generation) > staged_generation,
                "the retention boundary must be explicit",
            )?;
            report.record("retention=boundary-crossed");
        }
        other => return Err(format!("expected RetentionBoundaryCrossed, got {other:?}")),
    }

    let stats: ApplicationStorageStats = client
        .storage_stats()
        .await
        .map_err(|e| format!("storage_stats: {e}"))?;
    report.record(format!("operation_rows={}", stats.operation_rows));

    Ok(report)
}

// ----------------------------------------------------------------------------------------
// Operator Station-shaped bidirectional probe
// ----------------------------------------------------------------------------------------

/// A bidirectional Operator Station: attends addresses, replies, dispositions,
/// reconciles operations, follows deltas, and maintains its own store.
pub async fn run_operator_station_probe(config: &ProbeConfig) -> Result<ProbeReport, String> {
    let mut report = ProbeReport::default();

    let client = config.connect("fixture-station").await?;
    report.record(format!("store={}", client.logical_store_id().0));

    let primary = spec(
        &config.address("station-primary"),
        ApplicationCapability::Bidirectional,
        "operator station primary",
    );
    let secondary = spec(
        &config.address("station-secondary"),
        ApplicationCapability::Bidirectional,
        "operator station secondary",
    );
    let reporter = spec(
        &config.address("station-reporter"),
        ApplicationCapability::SendOnly,
        "operator station reporter",
    );

    // Multi-address lifecycle: attach the whole surface at once.
    let attached = client
        .attach(&[primary.clone(), secondary.clone(), reporter.clone()])
        .await;
    require(attached.ready, "station attach must be ready")?;
    require(
        attached.compensation.is_empty(),
        "a ready lifecycle owes no compensation",
    )?;
    report.record("lifecycle=attached");

    // Cancellation is safe: an untouched set is exactly reported.
    let cancelled = client
        .begin_reconcile_many(&[primary.clone(), secondary.clone()], RecoveryPolicy::Strict)
        .cancelled_outcome();
    let evidence: LifecycleCancellationEvidence = cancelled
        .cancellation
        .ok_or_else(|| "cancellation evidence is required".to_string())?;
    require(
        evidence.operation == LifecycleOperationKind::Reconcile
            && evidence.may_have_committed.is_none()
            && evidence.not_attempted.len() == 2,
        "cancellation must partition uncertain and untouched work",
    )?;
    report.record("lifecycle=cancellation-partitioned");

    // Compensation for a partially applied lifecycle: detach a live address
    // alongside one this client never attached.
    let unknown = config.address("station-unknown");
    let detached: MultiAddressOutcome = client
        .detach_many(&[secondary.address.clone(), unknown.clone()])
        .await;
    require(
        !detached.ready,
        "detaching an unattached address must not be ready",
    )?;
    let compensation = detached
        .compensation
        .iter()
        .find(|handle| handle.address == secondary.address)
        .ok_or_else(|| "compensation for the detached address is required".to_string())?;
    match &compensation.action {
        CompensationAction::Reattach(previous) => require(
            previous == &secondary,
            "compensation must restore the exact previous spec",
        )?,
        other => return Err(format!("expected Reattach(previous_spec), got {other:?}")),
    }
    report.record("lifecycle=compensable");

    // Reattach the secondary address for the rest of the probe.
    let restored = client.attach(std::slice::from_ref(&secondary)).await;
    require(restored.ready, "secondary reattach must be ready")?;

    // Inbound work arrives from the send-only reporter.
    let inbound = SendRequest {
        operation_id: OperationId(format!("station-inbound-{}", config.run_id)),
        sender: reporter.address.clone(),
        to: primary.address.clone(),
        cc: vec![secondary.address.clone()],
        kind: "request".to_string(),
        attention: "background".to_string(),
        requires_disposition: true,
        subject: Some("needs an operator".to_string()),
        body: "please handle this".to_string(),
        metadata: Some(r#"{"ticket":"fixture-1"}"#.to_string()),
        retry_budget: 1,
    };
    let inbound_sent = client
        .send(inbound.clone())
        .await
        .map_err(|e| format!("inbound send: {e}"))?;

    // Receive, then the caller's own durable-ingest boundary, then ack.
    let delivery: ReceivedDelivery = client
        .receive(&primary.address, Some(3000))
        .await
        .map_err(|e| format!("receive: {e}"))?
        .ok_or_else(|| "a delivery must be available".to_string())?;
    let identity: ExactDeliveryIdentity = delivery.delivery.clone();
    require(
        identity.message_id == inbound_sent.message_id && identity.recipient == primary.address,
        "receive must hand back the exact delivery identity",
    )?;
    require(
        delivery.metadata.as_deref() == inbound.metadata.as_deref(),
        "caller metadata must survive the round trip",
    )?;

    // The caller's durable ingest happens here (a real consumer would persist
    // its own work item). Only after that does it acknowledge.
    let ingested = format!(
        "ingested:{}:{}:{}",
        identity.message_id, identity.delivery_id, delivery.snapshot_version
    );
    report.record(ingested);
    let ack: AckResult = client
        .acknowledge(&delivery.ack)
        .await
        .map_err(|e| format!("acknowledge: {e}"))?;
    require(
        ack == AckResult::Marked,
        "the first acknowledgment after durable ingest must mark the delivery",
    )?;
    report.record("ack=marked");

    // Unresolved history, thread scoping, and the source of the work item.
    let unresolved: Vec<HistoryItem> = client
        .history(Some(primary.address.clone()), true, None, None, None, 50)
        .await
        .map_err(|e| format!("unresolved history: {e}"))?;
    require(
        unresolved
            .iter()
            .any(|item| item.message.id == inbound_sent.message_id),
        "an unresolved item must appear in unresolved history",
    )?;
    let source = SourceReference {
        logical_store_id: client.logical_store_id().clone(),
        message_id: inbound_sent.message_id,
    };
    let captured: MessageRow = match client
        .resolve_source(&source)
        .await
        .map_err(|e| format!("resolve_source: {e}"))?
    {
        SourceResolution::Authoritative(message) => message,
        other => return Err(format!("expected an authoritative source, got {other:?}")),
    };
    let foreign = SourceReference {
        logical_store_id: LogicalStoreId("store-v1-foreign".to_string()),
        message_id: inbound_sent.message_id,
    };
    match client
        .resolve_source_with_capture(&foreign, Some(captured.clone()))
        .await
        .map_err(|e| format!("resolve foreign source: {e}"))?
    {
        SourceResolution::Mismatch => {}
        other => return Err(format!("a foreign store must fail closed, got {other:?}")),
    }
    report.record("source=store-scoped");

    // Compound "Reply & Handle": the terminal disposition is fenced behind the
    // reply step, and the reply carries operator metadata.
    let compound = OperationId(format!("station-compound-{}", config.run_id));
    let steps = vec![
        CompoundStep {
            step_id: "reply".to_string(),
            position: 1,
            kind: "reply".to_string(),
            prerequisites: Vec::new(),
            declaration: serde_json::json!({"role": "reply"}),
        },
        CompoundStep {
            step_id: "handle".to_string(),
            position: 2,
            kind: "disposition".to_string(),
            prerequisites: vec!["reply".to_string()],
            declaration: serde_json::json!({"role": "terminal"}),
        },
    ];
    let declared: Vec<CompoundStepRecord> = client
        .declare_compound(&compound, &steps)
        .await
        .map_err(|e| format!("declare_compound: {e}"))?;
    require(declared.len() == 2, "both compound steps must be declared")?;

    let terminal = CompoundDispositionRequest {
        sender: primary.address.clone(),
        delivery: identity.clone(),
        state: "handled".to_string(),
        note: Some("handled by the operator fixture".to_string()),
        operation_id: compound.clone(),
        step_id: "handle".to_string(),
        outcome: Some(serde_json::json!({"handled": true})),
        recovery: None,
    };
    match client.complete_compound_disposition(&terminal).await {
        Err(ApplicationClientError::Partial(_)) => {}
        other => {
            return Err(format!(
                "the terminal step must be fenced behind its prerequisite, got {other:?}"
            ))
        }
    }

    let reply_request = ReplyRequest {
        operation_id: OperationId(format!("station-reply-{}", config.run_id)),
        sender: primary.address.clone(),
        message_id: inbound_sent.message_id,
        cc: Vec::new(),
        kind: "reply".to_string(),
        attention: "background".to_string(),
        requires_disposition: false,
        subject: Some("operator reply".to_string()),
        body: "on it".to_string(),
        metadata: Some(r#"{"operator":"fixture"}"#.to_string()),
        retry_budget: 1,
    };
    let reply_recovery = client
        .prepare_reply(&reply_request)
        .await
        .map_err(|e| format!("prepare_reply: {e}"))?;
    let replied = client
        .reply(reply_request)
        .await
        .map_err(|e| format!("reply: {e}"))?;
    require(
        replied.thread_id == inbound_sent.thread_id,
        "a reply must stay in the source thread",
    )?;
    require(
        reply_recovery.operation_id.0.starts_with("station-reply-"),
        "the reply recovery handle must name its operation",
    )?;
    let reply_step = client
        .complete_compound_step(
            &compound,
            "reply",
            CompoundStepState::Accepted,
            Some(&serde_json::json!({"message_id": replied.message_id})),
            None,
        )
        .await
        .map_err(|e| format!("complete reply step: {e}"))?;
    require(
        reply_step.state == "accepted",
        "the prerequisite step must record acceptance",
    )?;

    let disposition: DispositionRow = client
        .complete_compound_disposition(&terminal)
        .await
        .map_err(|e| format!("complete_compound_disposition: {e}"))?;
    require(
        disposition.message_id == inbound_sent.message_id
            && disposition.recipient == primary.address
            && disposition.state == "handled",
        "the disposition must name the exact recipient it resolved",
    )?;
    report.record("compound=reply-then-handle");

    // Threaded history now contains both the request and the reply.
    let threaded: Vec<HistoryItem> = client
        .history(
            Some(primary.address.clone()),
            false,
            Some(inbound_sent.thread_id),
            None,
            None,
            50,
        )
        .await
        .map_err(|e| format!("thread history: {e}"))?;
    require(
        threaded
            .iter()
            .all(|item| item.message.thread_id == inbound_sent.thread_id),
        "the thread filter must be exact",
    )?;
    let delivery_rows: Vec<Option<DeliveryRow>> =
        threaded.iter().map(|item| item.delivery.clone()).collect();
    report.record(format!("thread_items={}", delivery_rows.len()));

    // Health and principal provenance.
    let health = client.health().await.map_err(|e| format!("health: {e}"))?;
    let primary_health = health
        .iter()
        .find(|record| record.address == primary.address)
        .ok_or_else(|| "primary health record is missing".to_string())?;
    require(
        primary_health.registered && primary_health.receive_ready,
        "an attended bidirectional address must be receive-ready",
    )?;
    let provenance: &PrincipalProvenance = &primary_health.principal;
    require(
        matches!(
            provenance.verification,
            PrincipalVerification::Verified
                | PrincipalVerification::Unverified
                | PrincipalVerification::Unavailable
        ),
        "principal provenance must be an explicit typed claim",
    )?;
    report.record(format!("provenance={:?}", provenance.verification));

    // Delta stream: ordered, gap-detecting, and resyncable after a prune.
    let page: DeltaPage = client
        .delta_page(0, 200)
        .await
        .map_err(|e| format!("delta_page: {e}"))?;
    let deltas: &[StateDeltaRecord] = &page.deltas;
    require(!deltas.is_empty(), "the delta stream must carry evidence")?;
    for pair in deltas.windows(2) {
        require(
            pair[1].version == pair[0].version + 1,
            "delta versions must be contiguous",
        )?;
    }
    let observed = deltas[deltas.len() - 1].version;
    match client.delta_page(page.current_version + 25, 10).await {
        Err(ApplicationClientError::ResyncRequired { .. }) => {}
        other => {
            return Err(format!(
                "a cursor ahead of the store must require resync, got {other:?}"
            ))
        }
    }
    let maintenance = ApplicationStoreMaintenance::connect(config.backend.as_deref(), None)
        .await
        .map_err(|e| format!("maintenance connect: {e}"))?;
    require(
        maintenance.logical_store_id() == client.logical_store_id(),
        "maintenance must bind the same logical store",
    )?;
    let pruned: StoreDeltaCleanupReport = maintenance
        .cleanup_deltas(StoreDeltaRetentionPolicy {
            before_version: observed,
            max_delete: 1000,
        })
        .await
        .map_err(|e| format!("cleanup_deltas: {e}"))?;
    require(pruned.deltas_deleted > 0, "delta cleanup must prune")?;
    match client.delta_page(0, 200).await {
        Err(ApplicationClientError::ResyncRequired {
            expected_version, ..
        }) => {
            let resynced = client
                .delta_page(expected_version, 200)
                .await
                .map_err(|e| format!("resync delta_page: {e}"))?;
            require(
                resynced
                    .deltas
                    .iter()
                    .all(|delta| delta.version > expected_version),
                "resync must not regress",
            )?;
            report.record("delta=gap-detected-and-resynced");
        }
        Ok(page) => {
            require(
                page.retained_floor > 0,
                "a pruned stream must advertise its retained floor",
            )?;
            report.record("delta=floor-advertised");
        }
        other => return Err(format!("unexpected delta outcome: {other:?}")),
    }

    // Deliberate detach is durable, typed evidence rather than a silent stop.
    client
        .detach(&secondary.address)
        .await
        .map_err(|e| format!("detach: {e}"))?;
    let health = client
        .health()
        .await
        .map_err(|e| format!("health after detach: {e}"))?;
    let detached_record = health
        .iter()
        .find(|record| record.address == secondary.address)
        .ok_or_else(|| "detached address must still report health".to_string())?;
    require(
        detached_record.lifecycle.iter().any(|evidence| {
            matches!(
                evidence,
                ApplicationLifecycleEvidence::DeliberateDetach { .. }
            )
        }),
        "deliberate detach must be durable public evidence",
    )?;
    report.record("detach=deliberate");

    // Bounded store maintenance.
    let cleanup = client
        .cleanup(RetentionPolicy {
            completed_before_ms: i64::MAX,
            max_delete: 1,
        })
        .await
        .map_err(|e| format!("cleanup: {e}"))?;
    require(
        cleanup.operations_deleted <= 1,
        "cleanup must respect its bound",
    )?;
    report.record(format!("cleanup_deleted={}", cleanup.operations_deleted));

    Ok(report)
}

/// Compile-time proof that the supported contract types stay nameable from an
/// external, defaults-disabled consumer.
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
