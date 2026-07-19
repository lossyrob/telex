mod cli;
mod config;
mod courier;
mod model;
mod state;
mod toast;
mod tray;

use config::{AppConfig, LocalScope, RuntimeConfig};
use model::{
    DispositionRecord, FrontendThreadView, SentReceipt, SourceAvailability, SourceReferenceView,
    StationStateView,
};
use state::Runtime;
use std::sync::Arc;
use tauri::{Manager, RunEvent};

// Fixed frontend IPC contract (all serialized response fields are camelCase):
// commands: app_config, initial_state, read_thread, reply_to, set_disposition,
// retry_courier. Events: station-state (StationStateView) and station-delivery
// (StationMessage). Message bodies remain in Telex/in memory and are never
// persisted by this runtime.

#[tauri::command]
fn app_config(state: tauri::State<'_, Arc<Runtime>>) -> AppConfig {
    state.app_config()
}

#[tauri::command]
async fn initial_state(state: tauri::State<'_, Arc<Runtime>>) -> Result<StationStateView, String> {
    Ok(state.frontend_state().await)
}

#[tauri::command(rename_all = "camelCase")]
async fn read_thread(
    state: tauri::State<'_, Arc<Runtime>>,
    message_id: i64,
) -> Result<FrontendThreadView, String> {
    let thread = state.cli.read_full(message_id).await?;
    let mut sources = Vec::new();
    for reference in thread.message.source_references.clone() {
        let (resolution, message) = if reference.availability == SourceAvailability::Available {
            match state.cli.read_full(reference.id).await {
                Ok(source) => ("resolved".to_string(), Some(source.message)),
                Err(_) => ("unavailable-in-current-store".to_string(), None),
            }
        } else {
            ("unavailable-in-current-store".to_string(), None)
        };
        sources.push(SourceReferenceView {
            id: reference.id,
            thread_id: reference.thread_id,
            from: reference.from,
            to: reference.to.unwrap_or_else(|| "(unknown)".into()),
            subject: reference.subject,
            sent_at_ms: reference.sent_at_ms.unwrap_or(0),
            store_fingerprint: reference.store_fingerprint,
            resolution,
            message,
        });
    }
    Ok(FrontendThreadView {
        selected: thread.message.clone(),
        thread: thread.thread,
        sources,
        raw_metadata: thread.message.metadata_raw,
    })
}

#[tauri::command(rename_all = "camelCase")]
async fn reply_to(
    state: tauri::State<'_, Arc<Runtime>>,
    message_id: i64,
    body: String,
) -> Result<SentReceipt, String> {
    state.cli.reply(message_id, body).await
}

#[tauri::command(rename_all = "camelCase")]
async fn set_disposition(
    state: tauri::State<'_, Arc<Runtime>>,
    message_id: i64,
    disposition_state: String,
    note: Option<String>,
) -> Result<DispositionRecord, String> {
    let disposition = match disposition_state.as_str() {
        "deferred" | "defer" => "defer",
        "handled" | "handle" => "handle",
        "closed" | "close" => "close",
        _ => return Err("dispositionState must be deferred, handled, or closed".into()),
    };
    let record = state.cli.disposition(message_id, disposition, note).await?;
    state.apply_disposition(&record).await?;
    Ok(record)
}

#[tauri::command]
async fn retry_courier(state: tauri::State<'_, Arc<Runtime>>) -> Result<(), String> {
    state
        .set_courier(|courier| {
            courier.persistent = false;
            courier.detail = Some("manual courier retry requested".into());
        })
        .await;
    state.emit_state().await?;
    state.request_retry();
    Ok(())
}

pub fn run() {
    toast::set_process_aumid();
    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            app_config,
            initial_state,
            read_thread,
            reply_to,
            set_disposition,
            retry_courier,
        ])
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            let config = Arc::new(RuntimeConfig::from_env().map_err(io_error)?);
            let scope = LocalScope::load_or_create(&app_data_dir, &config).map_err(io_error)?;
            let runtime = Runtime::new(app.handle().clone(), config, scope);
            app.manage(runtime.clone());
            tray::setup(app.handle())?;

            if let Err(error) = toast::prepare(&app_data_dir) {
                let toast_runtime = runtime.clone();
                tauri::async_runtime::spawn(async move {
                    toast_runtime
                        .diagnostic("error", "toast-registration-failed", error.clone())
                        .await;
                    let _ = toast_runtime.set_toast_error(Some(error)).await;
                });
            }
            courier::spawn(runtime);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build operator-station-spike");

    app.run(|app_handle, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            let runtime = app_handle.state::<Arc<Runtime>>().inner().clone();
            if runtime.begin_shutdown() {
                api.prevent_exit();
                let handle = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    runtime
                        .set_courier(|courier| {
                            courier.phase = model::CourierPhase::Stopping;
                            courier.detail = Some("stopping courier and Station membership".into());
                        })
                        .await;
                    let _ = runtime.emit_state().await;
                    runtime.signal_shutdown();
                    if let Err(error) = runtime.cli.station_stop().await {
                        runtime
                            .diagnostic("error", "station-stop-failed", error)
                            .await;
                    }
                    handle.exit(0);
                });
            }
        }
    });
}

fn io_error(error: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, error)
}
