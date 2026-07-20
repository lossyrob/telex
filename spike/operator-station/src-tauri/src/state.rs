use crate::cli::TelexCli;
use crate::config::{AppConfig, LocalScope, RuntimeConfig};
use crate::model::{
    AddressOccupancy, AddressStatusView, CourierPhase, CourierState, Diagnostic, DispositionRecord,
    StationConfigView, StationMessage, StationRuntimeStatusView, StationStateView,
};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::sync::{watch, Notify, RwLock};

pub const STATE_EVENT: &str = "station-state";
pub const DELIVERY_EVENT: &str = "station-delivery";
const DIAGNOSTIC_LIMIT: usize = 200;

#[derive(Debug)]
struct StationData {
    startup_complete: bool,
    messages: BTreeMap<i64, StationMessage>,
    courier: CourierState,
    station_occupancy: Option<AddressOccupancy>,
    ingress_occupancy: Option<AddressOccupancy>,
    diagnostics: VecDeque<Diagnostic>,
    tested_telex_version: Option<String>,
    toast_error: Option<String>,
}

pub struct Runtime {
    app: AppHandle,
    pub config: Arc<RuntimeConfig>,
    pub cli: TelexCli,
    scope: Mutex<LocalScope>,
    data: RwLock<StationData>,
    retry: Notify,
    shutdown_tx: watch::Sender<bool>,
    shutting_down: AtomicBool,
}

#[derive(Debug)]
pub struct IngestResult {
    pub message: StationMessage,
    pub toast_eligible: bool,
}

impl Runtime {
    pub fn new(app: AppHandle, config: Arc<RuntimeConfig>, scope: LocalScope) -> Arc<Self> {
        let session_id = scope.session_id().to_string();
        let cli = TelexCli::new(config.clone(), session_id);
        let (shutdown_tx, _) = watch::channel(false);
        Arc::new(Self {
            app,
            config,
            cli,
            scope: Mutex::new(scope),
            data: RwLock::new(StationData {
                startup_complete: false,
                messages: BTreeMap::new(),
                courier: CourierState::default(),
                station_occupancy: None,
                ingress_occupancy: None,
                diagnostics: VecDeque::new(),
                tested_telex_version: None,
                toast_error: None,
            }),
            retry: Notify::new(),
            shutdown_tx,
            shutting_down: AtomicBool::new(false),
        })
    }

    pub fn app_config(&self) -> AppConfig {
        let scope = self.scope.lock().expect("local scope mutex poisoned");
        self.config.public(scope.session_id())
    }

    pub async fn frontend_state(&self) -> StationStateView {
        let data = self.data.read().await;
        let mut messages: Vec<_> = data.messages.values().cloned().collect();
        messages.sort_by_key(|message| std::cmp::Reverse(message.id));
        let courier_state = data.courier.phase.as_str().to_string();
        let phase = if data.startup_complete
            && matches!(
                data.courier.phase,
                CourierPhase::Armed
                    | CourierPhase::AckPending
                    | CourierPhase::Backoff
                    | CourierPhase::Paused
            ) {
            "running".to_string()
        } else {
            courier_state.clone()
        };
        let detail = data
            .toast_error
            .clone()
            .or_else(|| data.courier.detail.clone());
        StationStateView {
            config: StationConfigView {
                station_address: self.config.station_address.clone(),
                ingress_address: self.config.ingress_address.clone(),
                store_fingerprint: self.config.store_fingerprint.clone(),
                telex_version: data
                    .tested_telex_version
                    .clone()
                    .unwrap_or_else(|| "loading".into()),
            },
            messages,
            status: StationRuntimeStatusView {
                phase: phase.clone(),
                detail,
                courier_state,
                station: data.station_occupancy.as_ref().map(address_status_view),
                ingress: data.ingress_occupancy.as_ref().map(address_status_view),
                diagnostics: data
                    .diagnostics
                    .iter()
                    .map(|item| {
                        format!(
                            "{} {}: {}",
                            item.level.to_uppercase(),
                            item.code,
                            item.detail
                        )
                    })
                    .collect(),
            },
        }
    }

    pub async fn replace_startup_messages(&self, messages: Vec<StationMessage>) {
        let mut data = self.data.write().await;
        data.messages = messages
            .into_iter()
            .map(|message| (message.id, message))
            .collect();
    }

    pub async fn complete_startup(&self, high_water: i64) -> Result<(), String> {
        {
            let mut scope = self.scope.lock().expect("local scope mutex poisoned");
            scope.set_high_water(high_water)?;
        }
        {
            let mut data = self.data.write().await;
            data.startup_complete = true;
        }
        self.emit_state().await
    }

    pub async fn reset_startup(&self) {
        let mut data = self.data.write().await;
        data.startup_complete = false;
    }

    pub async fn ingest_live(&self, mut message: StationMessage) -> Result<IngestResult, String> {
        let prior_high_water = {
            let scope = self.scope.lock().expect("local scope mutex poisoned");
            scope.high_water()
        };
        let is_new = !self.data.read().await.messages.contains_key(&message.id);
        let toast_eligible = is_new
            && message.id > prior_high_water
            && message.toast_eligible(&self.config.station_address);

        {
            let mut data = self.data.write().await;
            if let Some(existing) = data.messages.get(&message.id) {
                message.ack_pending = existing.ack_pending;
            }
            data.messages.insert(message.id, message.clone());
        }
        {
            let mut scope = self.scope.lock().expect("local scope mutex poisoned");
            scope.set_high_water(message.id)?;
        }
        self.emit_state().await?;
        Ok(IngestResult {
            message,
            toast_eligible,
        })
    }

    pub async fn emit_delivery(&self, message: &StationMessage) -> Result<(), String> {
        self.app
            .emit(DELIVERY_EVENT, message)
            .map_err(|error| format!("emitting {DELIVERY_EVENT} failed: {error}"))
    }

    pub async fn mark_ack_pending(&self, message_id: i64, pending: bool) {
        let mut data = self.data.write().await;
        if let Some(message) = data.messages.get_mut(&message_id) {
            message.ack_pending = pending;
        }
    }

    pub async fn apply_disposition(&self, record: &DispositionRecord) -> Result<(), String> {
        let mut data = self.data.write().await;
        if let Some(message) = data.messages.get_mut(&record.message_id) {
            message.latest_disposition = Some(record.state.clone());
            message.actionable =
                !matches!(record.state.as_str(), "handled" | "closed" | "rejected");
        }
        drop(data);
        self.emit_state().await
    }

    pub async fn set_courier(&self, update: impl FnOnce(&mut CourierState)) {
        let mut data = self.data.write().await;
        update(&mut data.courier);
    }

    pub async fn set_version(&self, version: String) {
        self.data.write().await.tested_telex_version = Some(version);
    }

    pub async fn set_occupancy(
        &self,
        station: AddressOccupancy,
        ingress: AddressOccupancy,
    ) -> Result<(), String> {
        let mut data = self.data.write().await;
        data.station_occupancy = Some(station);
        data.ingress_occupancy = Some(ingress);
        drop(data);
        self.emit_state().await
    }

    pub async fn set_toast_error(&self, error: Option<String>) -> Result<(), String> {
        self.data.write().await.toast_error = error;
        self.emit_state().await
    }

    pub async fn diagnostic(&self, level: &str, code: &str, detail: impl Into<String>) {
        let mut detail = self.config.redact(&detail.into());
        if detail.len() > 1_000 {
            detail.truncate(1_000);
        }
        let mut data = self.data.write().await;
        if data.diagnostics.len() == DIAGNOSTIC_LIMIT {
            data.diagnostics.pop_front();
        }
        data.diagnostics.push_back(Diagnostic {
            at_ms: now_ms(),
            level: level.to_string(),
            code: code.to_string(),
            detail,
        });
    }

    pub async fn emit_state(&self) -> Result<(), String> {
        let snapshot = self.frontend_state().await;
        self.app
            .emit(STATE_EVENT, snapshot)
            .map_err(|error| format!("emitting {STATE_EVENT} failed: {error}"))
    }

    pub fn request_retry(&self) {
        self.retry.notify_one();
    }

    pub async fn wait_for_retry(&self) {
        self.retry.notified().await;
    }

    pub fn shutdown_receiver(&self) -> watch::Receiver<bool> {
        self.shutdown_tx.subscribe()
    }

    pub fn signal_shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
        self.retry.notify_waiters();
    }

    pub fn is_shutting_down(&self) -> bool {
        *self.shutdown_tx.borrow()
    }

    pub fn begin_shutdown(&self) -> bool {
        !self.shutting_down.swap(true, Ordering::SeqCst)
    }

    pub async fn daemon_hung_paused(&self) -> bool {
        let courier = &self.data.read().await.courier;
        courier.phase == CourierPhase::Paused
            && courier.persistent
            && courier.last_exit_code == Some(4)
    }
}

fn address_status_view(status: &AddressOccupancy) -> AddressStatusView {
    AddressStatusView {
        address: status.address.clone(),
        occupied: status.occupied,
        health: status.station_health.clone().unwrap_or_else(|| {
            if status.occupied {
                "occupied".into()
            } else {
                "unattended".into()
            }
        }),
        detail: status.error.clone(),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_are_bounded_without_message_log_storage() {
        assert_eq!(DIAGNOSTIC_LIMIT, 200);
        assert!(std::mem::size_of::<Diagnostic>() < 256);
    }
}
