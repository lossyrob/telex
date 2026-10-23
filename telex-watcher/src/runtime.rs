use crate::client::{bounded, Clients};
use crate::evidence::*;
use crate::management::{Command, LocalManagement, Request, Response};
use crate::process::{self, ContainmentIdentity, ProcessOutput, ProcessSpec};
use crate::protocol_v2::{self, AttemptRequest, DetectorRequest, Outcome, WatchRequest};
use crate::registry_v2::{now_ms, timestamp, Change, Refusal, Registry};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use telex::application_client::{ApplicationClientError, ApplicationDaemonBootstrap};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;
use uuid::Uuid;

/// Bootstrap authority is explicit. Production hosts use InstalledCurrent;
/// tests may supply ExactExecutable without any fallback.
pub struct RuntimeConfig {
    pub registry: PathBuf,
    pub daemon: ApplicationDaemonBootstrap,
    pub concurrency: usize,
    pub retention_warning_rows: u64,
    pub retention_warning_bytes: u64,
}

impl RuntimeConfig {
    pub fn installed(registry: PathBuf, trusted_root: PathBuf) -> Self {
        Self {
            registry,
            daemon: ApplicationDaemonBootstrap::InstalledCurrent { trusted_root },
            concurrency: 4,
            retention_warning_rows: 100_000,
            retention_warning_bytes: 256 * 1024 * 1024,
        }
    }
}

type SharedRegistry = Arc<Mutex<Registry>>;

pub struct Runtime {
    registry: SharedRegistry,
    id: String,
    clients: Clients,
    requests: mpsc::Receiver<Request>,
    concurrency: usize,
    attempts: JoinSet<CompletedAttempt>,
    cancellations: BTreeMap<String, oneshot::Sender<()>>,
    containment_directory: PathBuf,
    reconciled_registry_revision: Option<u64>,
    stopping: bool,
}

struct CompletedAttempt {
    watch: Watch,
    attempt: Attempt,
    output: Result<ProcessOutput, process::ProcessFailure>,
}

struct HeartbeatTask(Option<tokio::task::JoinHandle<()>>);

impl Drop for HeartbeatTask {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}

fn lock(registry: &SharedRegistry) -> Result<MutexGuard<'_, Registry>> {
    registry
        .lock()
        .map_err(|_| anyhow::anyhow!("watcher registry owner poisoned"))
}

impl Runtime {
    pub fn open(config: RuntimeConfig) -> Result<(Self, LocalManagement)> {
        anyhow::ensure!(
            (1..=16).contains(&config.concurrency),
            "concurrency must be 1..16"
        );
        let registry = Registry::open(
            &config.registry,
            config.retention_warning_rows,
            config.retention_warning_bytes,
        )?;
        let id = format!("watcher-runtime-{}", Uuid::new_v4());
        let containment_directory = registry.containment_directory();
        let clients = Clients::new(registry.responsibility.clone(), config.daemon);
        let (management, requests) = LocalManagement::channel();
        Ok((
            Self {
                registry: Arc::new(Mutex::new(registry)),
                id,
                clients,
                requests,
                concurrency: config.concurrency,
                attempts: JoinSet::new(),
                cancellations: BTreeMap::new(),
                containment_directory,
                reconciled_registry_revision: None,
                stopping: false,
            },
            management,
        ))
    }

    pub fn runtime_id(&self) -> &str {
        &self.id
    }

    /// Owns scheduling and administration until Shutdown or a registry failure.
    pub async fn run(mut self) -> Result<()> {
        let previous = lock(&self.registry)?.unfinished_containment()?;
        let mut proofs = Vec::new();
        for (_, evidence) in &previous {
            if let Some(value) = evidence {
                proofs.push((value.clone(), containment_ended(value).await));
            }
        }
        lock(&self.registry)?.start_runtime(&self.id, now_ms(), |evidence| {
            evidence
                .is_none_or(|value| proofs.iter().any(|(prior, ended)| prior == value && *ended))
        })?;
        let registry = self.registry.clone();
        let id = self.id.clone();
        let (heartbeat_error, mut heartbeat_failure) = mpsc::channel(1);
        let mut heartbeat = HeartbeatTask(Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                let update = (|| -> Result<()> {
                    let registry = lock(&registry)?;
                    let mut health = registry.runtime(&id)?.context("runtime record missing")?;
                    health.heartbeat_at = timestamp(now_ms());
                    health.registry_revision = registry.revision()?;
                    if health.reconciled_registry_revision != Some(health.registry_revision) {
                        health.production_ready = false;
                        if !health
                            .diagnostics
                            .contains(&RuntimeDiagnostic::MembershipReconciliationPending)
                        {
                            health
                                .diagnostics
                                .push(RuntimeDiagnostic::MembershipReconciliationPending);
                        }
                    }
                    health.retention = registry.retention(None)?;
                    registry.save_runtime(&health)
                })();
                if let Err(error) = update {
                    let _ = heartbeat_error.send(error).await;
                    break;
                }
            }
        })));
        let result = self.run_inner(&mut heartbeat_failure).await;
        self.stopping = true;
        for (_, cancel) in std::mem::take(&mut self.cancellations) {
            let _ = cancel.send(());
        }
        while let Some(completion) = self.attempts.join_next().await {
            match completion {
                Ok(completion) => {
                    let mut registry = lock(&self.registry)?;
                    registry.fail_attempt(
                        &completion.attempt.id,
                        DiagnosticCategory::RuntimeInterrupted,
                        "runtime shutting down",
                        now_ms(),
                    )?;
                    if completion
                        .output
                        .as_ref()
                        .is_err_and(|error| error.category() == "orphan-containment-unproven")
                    {
                        registry.block_watch(
                            &completion.watch.id,
                            BlockedReason::OrphanContainmentUnproven,
                        )?;
                    }
                }
                Err(error) => {
                    eprintln!("watcher attempt task failed during shutdown: {error}");
                }
            }
        }
        let detach = self.clients.shutdown().await;
        let heartbeat = heartbeat.0.take().context("heartbeat task missing")?;
        heartbeat.abort();
        match heartbeat.await {
            Ok(()) => {}
            Err(error) if error.is_cancelled() => {}
            Err(error) => return Err(error.into()),
        }
        {
            let registry = lock(&self.registry)?;
            let mut health = registry
                .runtime(&self.id)?
                .context("runtime record missing")?;
            health.status = RuntimeStatus::Stopped;
            health.production_ready = false;
            if result.is_err() {
                health.diagnostics.push(RuntimeDiagnostic::RegistryFailure);
            }
            health.heartbeat_at = timestamp(now_ms());
            health.detach = detach;
            registry.save_runtime(&health)?;
        }
        result
    }

    async fn run_inner(
        &mut self,
        heartbeat_failure: &mut mpsc::Receiver<anyhow::Error>,
    ) -> Result<()> {
        let mut tick = tokio::time::interval(Duration::from_millis(200));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut next_membership = 0_i64;
        while !self.stopping {
            tokio::select! {
                error = heartbeat_failure.recv() => {
                    return Err(error.context("heartbeat task stopped")?);
                }
                request = self.requests.recv() => {
                    if let Some(request) = request {
                        let response = self.manage(request.command).await;
                        // Losing a caller does not undo an admitted operation.
                        let _ = request.reply.send(response);
                    } else {
                        self.stopping = true;
                    }
                }
                completed = self.attempts.join_next(), if !self.attempts.is_empty() => {
                    let completed = completed.context("attempt completion missing")??;
                    self.cancellations.remove(&completed.watch.id);
                    self.complete_attempt(completed).await?;
                }
                _ = tick.tick() => {
                    let now = now_ms();
                    let revision = lock(&self.registry)?.revision()?;
                    if now >= next_membership || Some(revision) != self.reconciled_registry_revision || self.clients.reconciliation_needed {
                        let watches = lock(&self.registry)?.list()?;
                        self.reconciled_registry_revision = None;
                        self.refresh_health()?;
                        self.clients.reconcile(&watches).await;
                        for watch in watches {
                            if let Some(registration) = &watch.registration {
                                if let Some(client) = self.clients.get(&registration.backend_profile) {
                                    lock(&self.registry)?.bind_store(&watch.id, &client.logical_store_id().0)?;
                                }
                            }
                        }
                        self.reconciled_registry_revision = Some(revision);
                        next_membership = now_ms().saturating_add(30_000);
                    }
                    lock(&self.registry)?.coalesce(now)?;
                    let watches = lock(&self.registry)?.list()?;
                    for watch in &watches {
                        if watch.lifecycle == Lifecycle::Active {
                            if let Some(operation) = &watch.pending_operation_id {
                                self.recover(operation, false).await?;
                            }
                        }
                    }
                    self.launch_due()?;
                    self.refresh_health()?;
                }
            }
        }
        Ok(())
    }

    fn launch_due(&mut self) -> Result<()> {
        let now = now_ms();
        let watches = lock(&self.registry)?.list()?;
        for watch in watches {
            if self.attempts.len() >= self.concurrency {
                break;
            }
            let Some(registration) = &watch.registration else {
                continue;
            };
            if watch.lifecycle != Lifecycle::Active
                || watch.eligibility != Eligibility::Eligible
                || watch.pending_operation_id.is_some()
                || watch.running_attempt_id.is_some()
                || now < watch.next_attempt_ms
                || (!watch.coalesced_due && now < watch.next_due_ms)
                || !self.clients.ready(
                    &registration.backend_profile,
                    &registration.sender,
                    watch.logical_store_id.as_deref(),
                )
            {
                continue;
            }
            let (watch, attempt) = lock(&self.registry)?.begin_attempt(&watch.id, &self.id, now)?;
            let registration = watch
                .registration
                .as_ref()
                .context("v2 registration missing")?;
            let request = DetectorRequest {
                schema_version: 2,
                attempt: AttemptRequest {
                    id: attempt.id.clone(),
                    now: attempt.started_at.clone(),
                },
                watch: WatchRequest {
                    id: watch.id.clone(),
                    activated_at: watch.activated_at.clone(),
                    last_success_at: watch.last_success_at.clone(),
                    parameters: registration.parameters.clone(),
                },
                state: watch.state.clone(),
            };
            let bytes = protocol_v2::encode_request(&request)?;
            let spec = ProcessSpec {
                command: registration.command.clone(),
                working_directory: registration.working_directory.clone(),
                environment_allowlist: registration.environment_allowlist.clone(),
                timeout: Duration::from_secs(registration.timeout_seconds),
                containment_directory: self.containment_directory.clone(),
            };
            let (cancel, receive) = oneshot::channel();
            self.cancellations.insert(watch.id.clone(), cancel);
            let registry = self.registry.clone();
            let attempt_id = attempt.id.clone();
            self.attempts.spawn(async move {
                let output = process::execute(&spec, bytes, receive, move |identity| async move {
                    (|| -> Result<()> {
                        lock(&registry)?.containment(&attempt_id, serde_json::to_value(identity)?)
                    })()
                    .map_err(|error| error.to_string())
                })
                .await;
                CompletedAttempt {
                    watch,
                    attempt,
                    output,
                }
            });
        }
        Ok(())
    }

    async fn complete_attempt(&mut self, completion: CompletedAttempt) -> Result<()> {
        let CompletedAttempt {
            watch,
            attempt,
            output,
        } = completion;
        if !lock(&self.registry)?.attempt_is_current(&attempt)? {
            let mut registry = lock(&self.registry)?;
            registry.fail_attempt(
                &attempt.id,
                DiagnosticCategory::RuntimeInterrupted,
                "registration or lifecycle changed during attempt",
                now_ms(),
            )?;
            if output
                .as_ref()
                .is_err_and(|error| error.category() == "orphan-containment-unproven")
            {
                registry.block_watch(&watch.id, BlockedReason::OrphanContainmentUnproven)?;
            }
            return Ok(());
        }
        let output = match output {
            Ok(output) => output,
            Err(error) => {
                let mut registry = lock(&self.registry)?;
                registry.fail_attempt(
                    &attempt.id,
                    process_category(&error),
                    &error.to_string(),
                    now_ms(),
                )?;
                if error.category() == "orphan-containment-unproven" {
                    registry.block_watch(&watch.id, BlockedReason::OrphanContainmentUnproven)?;
                }
                return Ok(());
            }
        };
        let result = match protocol_v2::parse_result(&output.stdout) {
            Ok(result) => result,
            Err(error) => {
                return lock(&self.registry)?.fail_attempt(
                    &attempt.id,
                    protocol_category(&error),
                    &error.to_string(),
                    now_ms(),
                );
            }
        };
        if result.outcome == Outcome::Degraded {
            return lock(&self.registry)?.fail_attempt(
                &attempt.id,
                DiagnosticCategory::DetectorDegraded,
                "detector returned degraded",
                now_ms(),
            );
        }
        let next_state = result.next_state.unwrap_or_else(|| watch.state.clone());
        let terminal = result.outcome == Outcome::Terminal;
        if let Some(event) = result.event {
            let registration = watch
                .registration
                .as_ref()
                .context("registration missing")?;
            let client = self
                .clients
                .get(&registration.backend_profile)
                .context("client missing")?;
            let proposal = EventProposal {
                event,
                next_state,
                terminal,
            };
            let prepared =
                crate::staging::prepare(&client, &watch, &attempt, proposal, now_ms()).await;
            match prepared {
                Ok(operation) => {
                    lock(&self.registry)?.stage(&operation, now_ms())?;
                    self.recover(&operation.operation_id, false).await
                }
                Err(error) => lock(&self.registry)?.fail_attempt(
                    &attempt.id,
                    DiagnosticCategory::SendFailure,
                    &error.to_string(),
                    now_ms(),
                ),
            }
        } else {
            lock(&self.registry)?.finish_direct(&attempt.id, next_state, terminal, now_ms())
        }
    }

    async fn recover(&mut self, id: &str, manual: bool) -> Result<()> {
        if !manual && !lock(&self.registry)?.expire_recovery(id, now_ms())? {
            return Ok(());
        }
        let operation = lock(&self.registry)?
            .operation(id)?
            .context("pending operation missing")?;
        if crate::staging::verify(&operation).is_err() {
            lock(&self.registry)?.block_operation(id, BlockedReason::OperationIdentityConflict)?;
            return Ok(());
        }
        let profile = &operation.registration.backend_profile;
        let Some(client) = self.clients.get(profile) else {
            return Ok(());
        };
        if client.logical_store_id() != &operation.recovery.logical_store_id
            || client.responsibility() != &operation.recovery.responsibility
        {
            return lock(&self.registry)?.evidence(
                id,
                SendEvidence::Reconciliation(Err(ApplicationClientError::StoreBindingMismatch {
                    staged: operation.recovery.logical_store_id.clone(),
                    current: client.logical_store_id().clone(),
                })),
                now_ms(),
            );
        }
        let send = !manual
            && matches!(
                operation.status,
                OperationStatus::Staged | OperationStatus::RetryPermitted
            );
        if send
            && !self.clients.ready(
                profile,
                &operation.envelope.sender,
                Some(&operation.recovery.logical_store_id.0),
            )
        {
            return Ok(());
        }
        if !lock(&self.registry)?.reserve_recovery(id, send, manual, now_ms())? {
            return Ok(());
        }
        let evidence = if send {
            let result = bounded(client.send(operation.envelope.clone())).await;
            if let Err(error) = &result {
                if matches!(
                    error,
                    ApplicationClientError::MembershipLost { .. }
                        | ApplicationClientError::Collision(_)
                ) {
                    self.clients.invalidate(profile, error.clone());
                }
            }
            SendEvidence::send(result)
        } else {
            SendEvidence::Reconciliation(
                bounded(client.reconcile_operation(&operation.recovery)).await,
            )
        };
        lock(&self.registry)?.evidence(id, evidence, now_ms())
    }

    fn refresh_health(&self) -> Result<()> {
        let registry = lock(&self.registry)?;
        let mut health = registry
            .runtime(&self.id)?
            .context("runtime record missing")?;
        health.senders = self.clients.health.values().cloned().collect();
        health.retention = registry.retention(None)?;
        health.registry_revision = registry.revision()?;
        health.reconciled_registry_revision = self.reconciled_registry_revision;
        health.diagnostics.clear();
        let watches = registry.list()?;
        if self.reconciled_registry_revision != Some(health.registry_revision)
            || self.clients.reconciliation_needed
        {
            health
                .diagnostics
                .push(RuntimeDiagnostic::MembershipReconciliationPending);
        }
        for watch in &watches {
            let Some(registration) = &watch.registration else {
                continue;
            };
            if watch.lifecycle == Lifecycle::Removed && watch.pending_operation_id.is_none() {
                continue;
            }
            let ready = self.clients.ready(
                &registration.backend_profile,
                &registration.sender,
                watch.logical_store_id.as_deref(),
            );
            let position = health
                .senders
                .iter()
                .position(|sender| sender.backend_profile == registration.backend_profile);
            let sender = if let Some(position) = position {
                &mut health.senders[position]
            } else {
                health.senders.push(SenderHealth {
                    backend_profile: registration.backend_profile.clone(),
                    required_senders: Vec::new(),
                    logical_store_id: None,
                    ready: false,
                    membership: Vec::new(),
                    lifecycle: None,
                    compensation: Vec::new(),
                    error: None,
                });
                health
                    .senders
                    .last_mut()
                    .expect("inserted required sender profile")
            };
            sender.required_senders.push(registration.sender.clone());
            sender.ready &= ready;
        }
        for sender in &mut health.senders {
            sender.required_senders.sort();
            sender.required_senders.dedup();
        }
        if health.senders.iter().any(|sender| !sender.ready) {
            health.diagnostics.push(RuntimeDiagnostic::SenderUnready);
        }
        if health
            .senders
            .iter()
            .any(|sender| sender.lifecycle.as_ref().is_some_and(|l| !l.ready))
        {
            health
                .diagnostics
                .push(RuntimeDiagnostic::PartialMembership);
        }
        if health
            .senders
            .iter()
            .any(|sender| sender.membership.iter().any(|m| m.inbound_actionable > 0))
        {
            health
                .diagnostics
                .push(RuntimeDiagnostic::InboundActionableBacklog);
        }
        if health.retention.warning {
            health.diagnostics.push(RuntimeDiagnostic::RetentionWarning);
        }
        if watches
            .iter()
            .any(|watch| watch.blocked_reason == Some(BlockedReason::OrphanContainmentUnproven))
        {
            health
                .diagnostics
                .push(RuntimeDiagnostic::ContainmentUnproven);
        }
        health.production_ready = health.diagnostics.is_empty() && health.restart_reconciled;
        registry.save_runtime(&health)
    }

    async fn manage(&mut self, command: Command) -> Result<Response> {
        match command {
            Command::Register {
                json,
                working_directory,
            } => {
                let registration = match protocol_v2::parse_registration(&json, &working_directory)
                {
                    Ok(registration) => registration,
                    Err(error) => {
                        return self.refuse(
                            None,
                            "register",
                            Refusal::InvalidRegistration,
                            &error.to_string(),
                        )
                    }
                };
                Ok(Response::Mutation(
                    lock(&self.registry)?.register(registration, now_ms())?,
                ))
            }
            Command::List {
                include_removed,
                include_unsupported,
            } => Ok(Response::Watches(
                lock(&self.registry)?
                    .list()?
                    .into_iter()
                    .filter(|w| {
                        (include_removed || w.lifecycle != Lifecycle::Removed)
                            && (include_unsupported || w.schema_version == 2)
                    })
                    .collect(),
            )),
            Command::Show { id } => Ok(Response::Watch(lock(&self.registry)?.get(&id)?)),
            Command::Pause { id } => self.mutate(&id, Change::Pause),
            Command::Resume { id } => {
                let previous = lock(&self.registry)?.containment_for_resume(&id)?;
                if let Some(previous) = previous {
                    let ended = containment_ended(&previous).await;
                    lock(&self.registry)?.recheck_containment(&id, |_| ended)?;
                }
                self.mutate(&id, Change::Resume)
            }
            Command::Update {
                id,
                json,
                working_directory,
                state,
            } => {
                let registration = match protocol_v2::parse_registration(&json, &working_directory)
                {
                    Ok(registration) => registration,
                    Err(error) => {
                        return self.refuse(
                            Some(&id),
                            "update",
                            Refusal::InvalidRegistration,
                            &error.to_string(),
                        )
                    }
                };
                if let Some(value) = &state {
                    match protocol_v2::canonical_bytes(value) {
                        Ok(bytes) if bytes.len() <= 256 * 1024 => {}
                        Ok(_) => {
                            return self.refuse(
                                Some(&id),
                                "update",
                                Refusal::InvalidState,
                                "state replacement exceeds 256 KiB",
                            )
                        }
                        Err(error) => {
                            return self.refuse(
                                Some(&id),
                                "update",
                                Refusal::InvalidState,
                                &error.to_string(),
                            )
                        }
                    }
                }
                self.mutate(
                    &id,
                    Change::Update {
                        registration: Box::new(registration),
                        state,
                    },
                )
            }
            Command::GrantReconciliation {
                id,
                attempts,
                seconds,
            } => self.mutate(&id, Change::GrantReconciliation { attempts, seconds }),
            Command::GrantSendRetry {
                id,
                attempts,
                seconds,
            } => self.mutate(&id, Change::GrantSendRetry { attempts, seconds }),
            Command::CloseNotAccepted { id } => self.mutate(&id, Change::CloseNotAccepted),
            Command::Remove { id } => self.mutate(&id, Change::Remove),
            Command::ReconcileNow { id } => {
                let watch = lock(&self.registry)?.get(&id)?;
                let Some(watch) = watch else {
                    return self.refuse(
                        Some(&id),
                        "reconcile-now",
                        Refusal::NotFound,
                        "watch not found",
                    );
                };
                let Some(operation) = watch.pending_operation_id.as_deref() else {
                    return self.refuse(
                        Some(&id),
                        "reconcile-now",
                        Refusal::NoPendingOperation,
                        "watch has no pending operation",
                    );
                };
                let profile = &watch
                    .registration
                    .as_ref()
                    .context("pending registration missing")?
                    .backend_profile;
                if self.clients.get(profile).is_none() {
                    return self.refuse(
                        Some(&id),
                        "reconcile-now",
                        Refusal::RecoveryUnavailable,
                        "selected backend client is unavailable",
                    );
                }
                self.recover(operation, true).await?;
                Ok(Response::Mutation(
                    lock(&self.registry)?.recovery_result(&watch)?,
                ))
            }
            Command::Health => {
                self.refresh_health()?;
                Ok(Response::Health(
                    lock(&self.registry)?
                        .runtime(&self.id)?
                        .context("runtime missing")?,
                ))
            }
            Command::WatchHealth { id } => {
                let registry = lock(&self.registry)?;
                let watch = registry.get(&id)?.context("watch not found")?;
                let ready = watch.registration.as_ref().is_some_and(|r| {
                    self.clients.ready(
                        &r.backend_profile,
                        &r.sender,
                        watch.logical_store_id.as_deref(),
                    )
                });
                Ok(Response::WatchHealth(registry.health(&id, ready)?))
            }
            Command::Attempts { id, limit } => Ok(Response::Attempts(
                lock(&self.registry)?.attempts(&id, limit)?,
            )),
            Command::Events { id, limit } => {
                Ok(Response::Events(lock(&self.registry)?.events(&id, limit)?))
            }
            Command::RemovedEvidence { id, limit } => {
                let registry = lock(&self.registry)?;
                anyhow::ensure!(
                    registry
                        .get(&id)?
                        .is_some_and(|w| w.lifecycle == Lifecycle::Removed),
                    "removed-evidence requires a removed watch"
                );
                Ok(Response::Events(registry.events(&id, limit)?))
            }
            Command::Pending { id } => {
                let registry = lock(&self.registry)?;
                let watch = registry.get(&id)?.context("watch not found")?;
                let pending = watch
                    .pending_operation_id
                    .as_deref()
                    .map(|id| registry.operation(id))
                    .transpose()?
                    .flatten();
                Ok(Response::Pending(pending.map(Box::new)))
            }
            Command::ReceiptEvidence {
                operation_id,
                limit,
            } => Ok(Response::ReceiptEvidence(
                lock(&self.registry)?.receipt_evidence(&operation_id, limit)?,
            )),
            Command::Shutdown => {
                self.stopping = true;
                Ok(Response::Stopping)
            }
        }
    }

    fn refuse(
        &self,
        id: Option<&str>,
        action: &str,
        reason: Refusal,
        diagnostic: &str,
    ) -> Result<Response> {
        Ok(Response::Mutation(
            lock(&self.registry)?.refuse(id, action, reason, diagnostic)?,
        ))
    }

    fn mutate(&mut self, id: &str, change: Change) -> Result<Response> {
        let cancels = matches!(
            change,
            Change::Pause | Change::Remove | Change::Update { .. }
        );
        let result = lock(&self.registry)?.change(id, change, now_ms())?;
        if result.refusal.is_none() && cancels {
            if let Some(cancel) = self.cancellations.remove(id) {
                let _ = cancel.send(());
            }
        }
        Ok(Response::Mutation(result))
    }
}

fn process_category(error: &process::ProcessFailure) -> DiagnosticCategory {
    match error.category() {
        "timeout" => DiagnosticCategory::Timeout,
        "output-limit" => DiagnosticCategory::OutputLimit,
        "runtime-interrupted" => DiagnosticCategory::RuntimeInterrupted,
        _ => DiagnosticCategory::ProcessFailure,
    }
}

async fn containment_ended(value: &serde_json::Value) -> bool {
    match serde_json::from_value::<ContainmentIdentity>(value.clone()) {
        Ok(identity) => matches!(
            process::probe_previous(&identity).await,
            process::ContainmentStatus::Ended
        ),
        Err(_) => false,
    }
}

fn protocol_category(error: &protocol_v2::ProtocolError) -> DiagnosticCategory {
    match error {
        protocol_v2::ProtocolError::Malformed(_) => DiagnosticCategory::MalformedOutput,
        protocol_v2::ProtocolError::Canonicalization(_) => {
            DiagnosticCategory::CanonicalizationFailed
        }

        protocol_v2::ProtocolError::OutputLimit(_) => DiagnosticCategory::OutputLimit,
    }
}

#[cfg(test)]
#[path = "runtime_management_tests.rs"]
mod tests;
