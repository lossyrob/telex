use crate::cli::{WaitExecution, WaitPayload};
use crate::model::{AddressOccupancy, CourierPhase, StationMessage};
use crate::state::Runtime;
use crate::toast;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

const STATUS_INTERVAL: Duration = Duration::from_secs(5);
const ACK_BUDGET: Duration = Duration::from_secs(15);
const ACK_ATTEMPTS: usize = 3;
const ACK_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_ACK_PENDING_REDELIVERIES: u8 = 3;
const MAX_INGEST_FAILURES: u8 = 3;

pub fn spawn(runtime: Arc<Runtime>) {
    let courier = runtime.clone();
    tauri::async_runtime::spawn(async move {
        run(courier).await;
    });
    tauri::async_runtime::spawn(async move {
        status_loop(runtime).await;
    });
}

async fn run(runtime: Arc<Runtime>) {
    loop {
        if runtime.is_shutting_down() {
            break;
        }
        match startup(&runtime).await {
            Ok(()) => {}
            Err(error) => {
                runtime
                    .diagnostic("error", "startup-failed", error.clone())
                    .await;
                if !pause(&runtime, error, Some(1)).await {
                    break;
                }
                runtime.reset_startup().await;
                continue;
            }
        }

        courier_loop(&runtime).await;
        break;
    }
    runtime
        .set_courier(|courier| {
            courier.phase = CourierPhase::Stopped;
            courier.current_waiter_pid = None;
            courier.detail = None;
        })
        .await;
    let _ = runtime.emit_state().await;
}

async fn startup(runtime: &Arc<Runtime>) -> Result<(), String> {
    runtime
        .set_courier(|courier| {
            courier.phase = CourierPhase::Attaching;
            courier.detail = Some("attaching anchored Station membership".into());
            courier.persistent = false;
            courier.last_exit_code = None;
        })
        .await;
    runtime.emit_state().await?;
    runtime.cli.attach(std::process::id()).await?;

    let version = runtime.cli.version().await?;
    runtime
        .diagnostic("info", "telex-version", version.clone())
        .await;
    runtime.set_version(version).await;
    runtime
        .set_courier(|courier| {
            courier.phase = CourierPhase::Backfilling;
            courier.detail =
                Some("loading unresolved obligations and the recent 200-message tail".into());
        })
        .await;
    runtime.emit_state().await?;

    let (export, inbox) = tokio::try_join!(runtime.cli.export_history(), runtime.cli.inbox())?;
    let high_water = export
        .iter()
        .chain(inbox.iter())
        .map(|message| message.id)
        .max()
        .unwrap_or(0);
    let projection = startup_projection(export, inbox, &runtime.config.station_address);
    runtime.replace_startup_messages(projection).await;
    runtime.complete_startup(high_water).await?;
    runtime
        .diagnostic(
            "info",
            "startup-complete",
            format!("startup projection complete through message {high_water}"),
        )
        .await;
    Ok(())
}

fn startup_projection(
    export: Vec<StationMessage>,
    inbox: Vec<StationMessage>,
    station_address: &str,
) -> Vec<StationMessage> {
    let mut retained = BTreeMap::new();
    for message in export
        .into_iter()
        .filter(|message| message.is_unresolved_primary_disposition(station_address))
    {
        retained.insert(message.id, message);
    }
    for message in inbox {
        retained.insert(message.id, message);
    }
    retained.into_values().collect()
}

async fn courier_loop(runtime: &Arc<Runtime>) {
    let mut consecutive_daemon_hung = 0u8;
    let mut consecutive_ack_pending = 0u8;
    let mut consecutive_ingest_failures = 0u8;
    let mut ack_pending_message_id = None;
    let mut backoff_seconds = 1u64;
    loop {
        if runtime.is_shutting_down() {
            return;
        }
        if let CourierOperation::RetryAck(message_id) =
            next_courier_operation(ack_pending_message_id)
        {
            runtime
                .set_courier(|courier| {
                    courier.phase = CourierPhase::AckPending;
                    courier.detail = Some(format!(
                        "retrying acknowledgment for ingested message {message_id}"
                    ));
                    courier.ack_pending_message_id = Some(message_id);
                    courier.current_waiter_pid = None;
                    courier.persistent = false;
                })
                .await;
            let _ = runtime.emit_state().await;
            match ack_with_retry(runtime, message_id).await {
                Ok(()) => {
                    runtime.mark_ack_pending(message_id, false).await;
                    runtime
                        .set_courier(|courier| {
                            courier.phase = CourierPhase::Armed;
                            courier.detail =
                                Some(format!("message {message_id} acknowledgment recovered"));
                            courier.ack_pending_message_id = None;
                            courier.persistent = false;
                        })
                        .await;
                    let _ = runtime.emit_state().await;
                    ack_pending_message_id = None;
                    consecutive_ack_pending = 0;
                    backoff_seconds = 1;
                }
                Err(error) => {
                    consecutive_ack_pending = consecutive_ack_pending.saturating_add(1);
                    let detail = format!(
                        "ack for message {message_id} remains pending after direct retry {consecutive_ack_pending}/{MAX_ACK_PENDING_REDELIVERIES}: {error}"
                    );
                    runtime
                        .diagnostic("error", "ack-pending-retry-failed", detail.clone())
                        .await;
                    if failure_decision(consecutive_ack_pending, MAX_ACK_PENDING_REDELIVERIES)
                        == FailureDecision::Pause
                    {
                        if !pause(runtime, detail, Some(1)).await {
                            return;
                        }
                        consecutive_ack_pending = 0;
                        backoff_seconds = 1;
                    } else {
                        runtime
                            .set_courier(|courier| {
                                courier.phase = CourierPhase::AckPending;
                                courier.detail = Some(detail);
                                courier.ack_pending_message_id = Some(message_id);
                            })
                            .await;
                        let _ = runtime.emit_state().await;
                        if !delay_retry_or_shutdown(runtime, Duration::from_secs(backoff_seconds))
                            .await
                        {
                            return;
                        }
                        backoff_seconds = (backoff_seconds * 2).min(5);
                    }
                }
            }
            continue;
        }
        runtime
            .set_courier(|courier| {
                courier.phase = CourierPhase::Armed;
                courier.detail = Some("waiting for live delivery".into());
                courier.persistent = false;
                courier.consecutive_daemon_hung = consecutive_daemon_hung;
                courier.current_waiter_pid = None;
                courier.ack_pending_message_id = ack_pending_message_id;
            })
            .await;
        let _ = runtime.emit_state().await;

        let wait = match runtime.cli.spawn_wait() {
            Ok(wait) => wait,
            Err(error) => {
                runtime
                    .diagnostic("error", "wait-spawn-failed", error.clone())
                    .await;
                if !pause(runtime, error, Some(1)).await {
                    return;
                }
                continue;
            }
        };
        let waiter_pid = wait.pid();
        runtime
            .set_courier(|courier| courier.current_waiter_pid = waiter_pid)
            .await;
        let execution = wait.finish(runtime.shutdown_receiver()).await;
        runtime
            .set_courier(|courier| courier.current_waiter_pid = None)
            .await;
        let execution = match execution {
            Ok(WaitExecution::Shutdown) => return,
            Ok(execution) => execution,
            Err(error) => {
                runtime
                    .diagnostic("error", "wait-failed", error.clone())
                    .await;
                if !pause(runtime, error, Some(1)).await {
                    return;
                }
                continue;
            }
        };
        let WaitExecution::Exited {
            code,
            stdout,
            stderr,
        } = execution
        else {
            return;
        };
        runtime
            .set_courier(|courier| courier.last_exit_code = Some(code))
            .await;

        let (decision, next_hung) = decide_exit(code, consecutive_daemon_hung);
        consecutive_daemon_hung = next_hung;
        match decision {
            ExitDecision::Delivery => {
                consecutive_daemon_hung = 0;
                match process_delivery(runtime, &stdout).await {
                    Ok(DeliveryOutcome::Acked) => {
                        consecutive_ack_pending = 0;
                        consecutive_ingest_failures = 0;
                        ack_pending_message_id = None;
                        backoff_seconds = 1;
                    }
                    Ok(DeliveryOutcome::AckPending(message_id)) => {
                        consecutive_ingest_failures = 0;
                        ack_pending_message_id = Some(message_id);
                        consecutive_ack_pending = 1;
                        if !delay_retry_or_shutdown(runtime, Duration::from_secs(1)).await {
                            return;
                        }
                    }
                    Err(error) => {
                        consecutive_ack_pending = 0;
                        ack_pending_message_id = None;
                        consecutive_ingest_failures = consecutive_ingest_failures.saturating_add(1);
                        runtime
                            .diagnostic("error", "delivery-ingest-failed", error.clone())
                            .await;
                        let detail = format!(
                            "delivery ingest failed {consecutive_ingest_failures}/{MAX_INGEST_FAILURES}: {error}"
                        );
                        if failure_decision(consecutive_ingest_failures, MAX_INGEST_FAILURES)
                            == FailureDecision::Pause
                        {
                            if !pause(runtime, detail, Some(1)).await {
                                return;
                            }
                            consecutive_ingest_failures = 0;
                            backoff_seconds = 1;
                        } else {
                            runtime
                                .set_courier(|courier| {
                                    courier.phase = CourierPhase::Backoff;
                                    courier.detail = Some(detail);
                                })
                                .await;
                            let _ = runtime.emit_state().await;
                            if !delay_or_shutdown(runtime, Duration::from_secs(backoff_seconds))
                                .await
                            {
                                return;
                            }
                            backoff_seconds = (backoff_seconds * 2).min(5);
                        }
                    }
                }
            }
            ExitDecision::Rearm => {
                consecutive_daemon_hung = 0;
                consecutive_ingest_failures = 0;
                backoff_seconds = 1;
            }
            ExitDecision::Reattach => {
                consecutive_daemon_hung = 0;
                consecutive_ingest_failures = 0;
                runtime
                    .set_courier(|courier| {
                        courier.phase = CourierPhase::Attaching;
                        courier.detail =
                            Some(format!("wait exited {code}; restoring anchored membership"));
                    })
                    .await;
                let _ = runtime.emit_state().await;
                if let Err(error) = runtime.cli.attach(std::process::id()).await {
                    runtime
                        .diagnostic("error", "reattach-failed", error.clone())
                        .await;
                    if !pause(runtime, error, Some(1)).await {
                        return;
                    }
                } else if !delay_or_shutdown(runtime, Duration::from_secs(backoff_seconds)).await {
                    return;
                }
            }
            ExitDecision::Backoff => {
                let detail = if stderr.trim().is_empty() {
                    format!("telex wait exited {code}; retrying after recovery backoff")
                } else {
                    format!(
                        "telex wait exited {code}: {}",
                        runtime.config.redact(stderr.trim())
                    )
                };
                runtime
                    .set_courier(|courier| {
                        courier.phase = CourierPhase::Backoff;
                        courier.detail = Some(detail);
                        courier.consecutive_daemon_hung = consecutive_daemon_hung;
                    })
                    .await;
                let _ = runtime.emit_state().await;
                if !delay_or_shutdown(runtime, Duration::from_secs(backoff_seconds)).await {
                    return;
                }
                backoff_seconds = (backoff_seconds * 2).min(5);
            }
            ExitDecision::Pause => {
                let detail = if code == 4 {
                    "telex wait reported daemon-hung twice; paused until status recovery or manual retry"
                        .to_string()
                } else if stderr.trim().is_empty() {
                    format!("telex wait exited {code}; manual retry required")
                } else {
                    format!(
                        "telex wait exited {code}: {}",
                        runtime.config.redact(stderr.trim())
                    )
                };
                runtime
                    .diagnostic("error", "courier-paused", detail.clone())
                    .await;
                if !pause(runtime, detail, Some(code)).await {
                    return;
                }
                consecutive_daemon_hung = 0;
                consecutive_ingest_failures = 0;
                backoff_seconds = 1;
            }
        }
    }
}

async fn process_delivery(runtime: &Arc<Runtime>, stdout: &str) -> Result<DeliveryOutcome, String> {
    let payload = runtime.cli.parse_wait_payload(stdout)?;
    validate_wait_payload(&payload, &runtime.config.station_address)?;
    let mut progress = DeliveryProgress::new(payload.id);
    let thread = runtime.cli.read_full(payload.id).await?;
    let ingest = runtime.ingest_live(thread.message).await?;
    progress.mark_ingested()?;

    if ingest.toast_eligible {
        match toast::show(&ingest.message) {
            Ok(()) => {
                runtime.set_toast_error(None).await?;
            }
            Err(error) => {
                runtime
                    .diagnostic("error", "toast-failed", error.clone())
                    .await;
                runtime.set_toast_error(Some(error)).await?;
            }
        }
    }
    runtime.emit_delivery(&ingest.message).await?;
    progress.mark_emitted()?;

    match ack_with_retry(runtime, payload.id).await {
        Ok(()) => {
            progress.mark_acked()?;
            runtime.mark_ack_pending(payload.id, false).await;
            runtime
                .set_courier(|courier| {
                    courier.phase = CourierPhase::Armed;
                    courier.detail =
                        Some(format!("message {} ingested and acknowledged", payload.id));
                    courier.ack_pending_message_id = None;
                    courier.persistent = false;
                })
                .await;
            runtime.emit_state().await?;
            Ok(DeliveryOutcome::Acked)
        }
        Err(error) => {
            runtime.mark_ack_pending(payload.id, true).await;
            runtime
                .set_courier(|courier| {
                    courier.phase = CourierPhase::AckPending;
                    courier.detail = Some(error.clone());
                    courier.ack_pending_message_id = Some(payload.id);
                    courier.persistent = false;
                })
                .await;
            runtime.diagnostic("error", "ack-pending", error).await;
            runtime.emit_state().await?;
            Ok(DeliveryOutcome::AckPending(payload.id))
        }
    }
}

fn validate_wait_payload(payload: &WaitPayload, station_address: &str) -> Result<(), String> {
    if payload.id < 1
        || payload.thread_id < 1
        || payload.to.trim().is_empty()
        || payload.delivered_to != station_address
        || payload.primary_to.trim().is_empty()
        || payload.kind.trim().is_empty()
        || payload.attention.trim().is_empty()
        || payload.sent_at_ms < 0
        || payload.buffered_at_ms < 0
        || payload.waiter_exit_ms < 0
        || payload.lease_epoch < 1
    {
        return Err("telex wait exit-0 payload failed required-field validation; not acked".into());
    }
    Ok(())
}

async fn ack_with_retry(runtime: &Arc<Runtime>, message_id: i64) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + ACK_BUDGET;
    let mut errors = Vec::new();
    for attempt in 1..=ACK_ATTEMPTS {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match runtime
            .cli
            .ack(message_id, remaining.min(ACK_ATTEMPT_TIMEOUT))
            .await
        {
            Ok(()) => return Ok(()),
            Err(error) => errors.push(format!("attempt {attempt}: {error}")),
        }
        if attempt < ACK_ATTEMPTS {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1).min(remaining)).await;
        }
    }
    Err(format!(
        "ack for message {message_id} remains pending after {ACK_ATTEMPTS} attempts/15s: {}",
        errors.join("; ")
    ))
}

async fn pause(runtime: &Arc<Runtime>, detail: String, exit_code: Option<i32>) -> bool {
    let mut pause_detail = detail;
    loop {
        if runtime.is_shutting_down() {
            return false;
        }
        runtime
            .set_courier(|courier| {
                courier.phase = CourierPhase::Paused;
                courier.detail = Some(pause_detail.clone());
                courier.persistent = true;
                courier.current_waiter_pid = None;
                courier.last_exit_code = exit_code;
            })
            .await;
        let _ = runtime.emit_state().await;
        let mut shutdown = runtime.shutdown_receiver();
        let retry = tokio::select! {
            _ = runtime.wait_for_retry() => !runtime.is_shutting_down(),
            _ = shutdown.changed() => false,
        };
        if !retry {
            return false;
        }
        runtime
            .set_courier(|courier| {
                courier.phase = CourierPhase::Attaching;
                courier.detail = Some("retrying anchored Station membership".into());
                courier.persistent = false;
            })
            .await;
        let _ = runtime.emit_state().await;
        match runtime.cli.attach(std::process::id()).await {
            Ok(()) => return true,
            Err(error) => {
                runtime
                    .diagnostic("error", "retry-attach-failed", error.clone())
                    .await;
                pause_detail = error;
            }
        }
    }
}

async fn delay_or_shutdown(runtime: &Arc<Runtime>, delay: Duration) -> bool {
    if runtime.is_shutting_down() {
        return false;
    }
    let mut shutdown = runtime.shutdown_receiver();
    tokio::select! {
        _ = tokio::time::sleep(delay) => true,
        _ = shutdown.changed() => false,
    }
}

async fn delay_retry_or_shutdown(runtime: &Arc<Runtime>, delay: Duration) -> bool {
    if runtime.is_shutting_down() {
        return false;
    }
    let mut shutdown = runtime.shutdown_receiver();
    tokio::select! {
        _ = tokio::time::sleep(delay) => true,
        _ = runtime.wait_for_retry() => !runtime.is_shutting_down(),
        _ = shutdown.changed() => false,
    }
}

async fn status_loop(runtime: Arc<Runtime>) {
    let mut interval = tokio::time::interval(STATUS_INTERVAL);
    let mut shutdown = runtime.shutdown_receiver();
    loop {
        tokio::select! {
            _ = interval.tick() => {
                if runtime.is_shutting_down() {
                    break;
                }
                refresh_status(&runtime).await;
            }
            _ = shutdown.changed() => break,
        }
    }
}

async fn refresh_status(runtime: &Arc<Runtime>) {
    let (station_result, ingress_result, station_runtime_result) = tokio::join!(
        runtime.cli.address_status(&runtime.config.station_address),
        runtime.cli.address_status(&runtime.config.ingress_address),
        runtime.cli.station_status(),
    );
    let station_runtime_recovered = station_runtime_result.is_ok();
    let mut station = result_or_error(&runtime.config.station_address, station_result, runtime);
    if let Ok(detail) = station_runtime_result {
        station.station_health = detail.station_health;
        station.pending_unconsumed_count = detail.pending_unconsumed_count;
        station.live_waiters_count = detail.live_waiters_count;
    }
    let ingress = result_or_error(&runtime.config.ingress_address, ingress_result, runtime);
    let _ = runtime.set_occupancy(station, ingress).await;
    if station_runtime_recovered && runtime.daemon_hung_paused().await {
        runtime
            .diagnostic(
                "info",
                "daemon-recovered",
                "station status succeeded; resuming paused courier",
            )
            .await;
        runtime.request_retry();
    }
}

fn result_or_error(
    address: &str,
    result: Result<AddressOccupancy, String>,
    runtime: &Runtime,
) -> AddressOccupancy {
    result.unwrap_or_else(|error| AddressOccupancy {
        address: address.to_string(),
        occupied: false,
        age_secs: 0.0,
        occupant: None,
        station_health: None,
        pending_unconsumed_count: None,
        live_waiters_count: None,
        error: Some(runtime.config.redact(&error)),
        refreshed_at_ms: now_ms(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExitDecision {
    Delivery,
    Rearm,
    Reattach,
    Backoff,
    Pause,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailureDecision {
    Backoff,
    Pause,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeliveryOutcome {
    Acked,
    AckPending(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CourierOperation {
    Wait,
    RetryAck(i64),
}

fn next_courier_operation(ack_pending_message_id: Option<i64>) -> CourierOperation {
    match ack_pending_message_id {
        Some(message_id) => CourierOperation::RetryAck(message_id),
        None => CourierOperation::Wait,
    }
}

fn failure_decision(attempt: u8, maximum: u8) -> FailureDecision {
    if attempt >= maximum {
        FailureDecision::Pause
    } else {
        FailureDecision::Backoff
    }
}

fn decide_exit(code: i32, consecutive_daemon_hung: u8) -> (ExitDecision, u8) {
    match code {
        0 => (ExitDecision::Delivery, 0),
        1 => (ExitDecision::Pause, 0),
        2 => (ExitDecision::Rearm, 0),
        3 | 5 => (ExitDecision::Reattach, 0),
        4 if consecutive_daemon_hung >= 1 => (ExitDecision::Pause, 2),
        4 => (ExitDecision::Backoff, 1),
        _ => (ExitDecision::Pause, 0),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeliveryStage {
    Read,
    Ingested,
    Emitted,
    Acked,
}

struct DeliveryProgress {
    message_id: i64,
    stage: DeliveryStage,
}

impl DeliveryProgress {
    fn new(message_id: i64) -> Self {
        Self {
            message_id,
            stage: DeliveryStage::Read,
        }
    }

    fn mark_ingested(&mut self) -> Result<(), String> {
        self.transition(DeliveryStage::Read, DeliveryStage::Ingested)
    }

    fn mark_emitted(&mut self) -> Result<(), String> {
        self.transition(DeliveryStage::Ingested, DeliveryStage::Emitted)
    }

    fn mark_acked(&mut self) -> Result<(), String> {
        self.transition(DeliveryStage::Emitted, DeliveryStage::Acked)
    }

    fn transition(&mut self, expected: DeliveryStage, next: DeliveryStage) -> Result<(), String> {
        if self.stage != expected {
            return Err(format!(
                "delivery {} ordering violation: {:?} cannot advance to {:?}",
                self.message_id, self.stage, next
            ));
        }
        self.stage = next;
        Ok(())
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: i64, required: bool, latest: Option<&str>) -> StationMessage {
        StationMessage {
            id,
            thread_id: id,
            parent_id: None,
            from: Some("attention:rob".into()),
            to: "operator:rob".into(),
            cc: vec![],
            kind: "note".into(),
            attention: "background".into(),
            requires_disposition: required,
            subject: None,
            body: "body".into(),
            metadata_raw: None,
            sent_at_ms: id,
            created_at_ms: Some(id),
            delivered_to: "operator:rob".into(),
            primary_to: "operator:rob".into(),
            delivery_role: "to".into(),
            requires_disposition_for_current_recipient: required,
            latest_disposition: latest.map(str::to_string),
            actionable: required,
            ack_pending: false,
            source_references: vec![],
            metadata_error: None,
        }
    }

    #[test]
    fn startup_keeps_all_unresolved_plus_recent_two_hundred() {
        let export: Vec<_> = (1..=250)
            .map(|id| {
                if id == 1 {
                    message(id, true, None)
                } else {
                    message(id, false, None)
                }
            })
            .collect();
        let inbox: Vec<_> = (51..=250).map(|id| message(id, false, None)).collect();
        let projected = startup_projection(export, inbox, "operator:rob");
        assert_eq!(projected.len(), 201);
        assert!(projected.iter().any(|message| message.id == 1));
        assert!(projected.iter().any(|message| message.id == 250));
        assert!(!projected.iter().any(|message| message.id == 2));
    }

    #[test]
    fn courier_exit_decisions_match_recovery_contract() {
        assert_eq!(decide_exit(0, 0), (ExitDecision::Delivery, 0));
        assert_eq!(decide_exit(1, 0), (ExitDecision::Pause, 0));
        assert_eq!(decide_exit(2, 0), (ExitDecision::Rearm, 0));
        assert_eq!(decide_exit(3, 0), (ExitDecision::Reattach, 0));
        assert_eq!(decide_exit(5, 0), (ExitDecision::Reattach, 0));
        assert_eq!(decide_exit(4, 0), (ExitDecision::Backoff, 1));
        assert_eq!(decide_exit(4, 1), (ExitDecision::Pause, 2));
    }

    #[test]
    fn repeated_delivery_failures_escalate_after_a_bounded_count() {
        assert_eq!(
            failure_decision(1, MAX_ACK_PENDING_REDELIVERIES),
            FailureDecision::Backoff
        );
        assert_eq!(
            failure_decision(MAX_ACK_PENDING_REDELIVERIES, MAX_ACK_PENDING_REDELIVERIES),
            FailureDecision::Pause
        );
        assert_eq!(
            failure_decision(MAX_INGEST_FAILURES, MAX_INGEST_FAILURES),
            FailureDecision::Pause
        );
    }

    #[test]
    fn ack_pending_retries_saved_ack_without_waiter_redelivery() {
        let message_id = 42;
        assert_eq!(
            next_courier_operation(Some(message_id)),
            CourierOperation::RetryAck(message_id)
        );
        assert_eq!(
            failure_decision(MAX_ACK_PENDING_REDELIVERIES, MAX_ACK_PENDING_REDELIVERIES),
            FailureDecision::Pause
        );
        assert_eq!(
            next_courier_operation(Some(message_id)),
            CourierOperation::RetryAck(message_id)
        );
        assert_eq!(next_courier_operation(None), CourierOperation::Wait);
    }

    #[test]
    fn three_ack_attempts_fit_inside_the_fifteen_second_budget() {
        let command_time = ACK_ATTEMPT_TIMEOUT * ACK_ATTEMPTS as u32;
        let retry_sleep = Duration::from_secs((ACK_ATTEMPTS - 1) as u64);
        assert!(command_time + retry_sleep <= ACK_BUDGET);
    }

    #[test]
    fn delivery_cannot_ack_before_ingest_and_frontend_emit() {
        let mut progress = DeliveryProgress::new(7);
        assert!(progress.mark_acked().is_err());
        progress.mark_ingested().unwrap();
        assert!(progress.mark_acked().is_err());
        progress.mark_emitted().unwrap();
        progress.mark_acked().unwrap();
        assert_eq!(progress.stage, DeliveryStage::Acked);
    }
}
