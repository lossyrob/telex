//! macOS desktop shell for the Station prototype. All product behavior lives in
//! `station-core`; this file only maps engine events onto the desktop: native
//! notifications, the menu-bar item, the dock badge and "demand attention".

use station_core::engine::Engine;
use station_core::model::{Counts, NotificationRecord, Snapshot, StationEvent, Tier};
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, UserAttentionType, WindowEvent};
use tauri_plugin_notification::NotificationExt;

struct Shell {
    engine: Arc<Engine>,
    /// The thread behind the most recent banner, so activating the app from a
    /// notification lands on the thing that asked for attention.
    last_banner: Mutex<Option<(i64, std::time::Instant)>>,
}

#[tauri::command]
async fn station_call(
    state: tauri::State<'_, Arc<Shell>>,
    cmd: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if std::env::var_os("STATION_DEBUG").is_some() {
        eprintln!("ui -> engine: {cmd}");
    }
    state.engine.call(&cmd, args).await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn shell_action(action: String, args: serde_json::Value) -> Result<(), String> {
    match action.as_str() {
        // Only ever reached after the user confirmed the destination in the UI.
        "open_url" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or_default();
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err("only http(s) links can be opened".into());
            }
            std::process::Command::new("open").arg(url).spawn().map(|_| ()).map_err(|e| e.to_string())
        }
        other => Err(format!("unknown shell action {other}")),
    }
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn tier_word(tier: Tier) -> &'static str {
    match tier {
        Tier::Blocked => "Blocked",
        Tier::NeedsReply => "Needs reply",
        Tier::Routine => "Routine",
        Tier::Fyi => "FYI",
    }
}

/// The menu-bar item is the quiet channel: a count that is always current, and
/// the top of the queue one click away.
fn rebuild_tray(app: &AppHandle, snap: &Snapshot) -> tauri::Result<()> {
    let Some(tray) = app.tray_by_id("station") else { return Ok(()) };
    let c: &Counts = &snap.counts;
    let title = if c.blocked > 0 {
        format!("{} blocked", c.blocked)
    } else if c.needs_me > 0 {
        format!("{}", c.needs_me)
    } else {
        String::new()
    };
    tray.set_title(Some(title))?;
    tray.set_tooltip(Some(format!(
        "Telex Station: {} need you ({} blocked, {} need a reply, {} routine)",
        c.needs_me, c.blocked, c.needs_reply, c.routine
    )))?;

    let menu = Menu::new(app)?;
    let summary = if c.needs_me == 0 {
        "Nothing needs you".to_string()
    } else {
        format!("{} need you · {} blocked", c.needs_me, c.blocked)
    };
    menu.append(&MenuItem::with_id(app, "summary", summary, false, None::<&str>)?)?;
    let mut top: Vec<_> = snap.threads.iter().filter(|t| t.status == "needs-me").collect();
    top.sort_by(|a, b| b.tier.cmp(&a.tier).then(a.last_inbound_ms.cmp(&b.last_inbound_ms)));
    for t in top.iter().take(6) {
        let name = snap
            .agents
            .iter()
            .find(|a| a.address == t.agent)
            .map(|a| a.display_name.clone())
            .unwrap_or_else(|| t.agent.clone());
        let subject: String = t.subject.chars().take(48).collect();
        let label = format!("{}  ·  {name}: {subject}", tier_word(t.tier));
        menu.append(&MenuItem::with_id(app, format!("thread:{}", t.thread_id), label, true, None::<&str>)?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "open", "Open Station", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "focus-blocked", "Only Blocked may interrupt for 1 hour", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "focus-off", "Interruptions: normal", true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "Quit Station (stops attending)", true, None::<&str>)?)?;
    tray.set_menu(Some(menu))?;

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_badge_count(if c.badge > 0 { Some(c.badge as i64) } else { None });
    }
    Ok(())
}

fn deliver(app: &AppHandle, shell: &Shell, record: &NotificationRecord) {
    if !record.banner {
        return;
    }
    let outcome = app.notification().builder().title(&record.title).body(&record.body).show();
    if let Err(error) = outcome {
        eprintln!("notification submit failed: {error}");
    }
    if record.sound {
        // A distinct system sound for the tiers allowed to make noise.
        let sound = if record.demand { "Sosumi" } else { "Tink" };
        let _ = std::process::Command::new("afplay")
            .arg(format!("/System/Library/Sounds/{sound}.aiff"))
            .spawn();
    }
    if record.demand {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.request_user_attention(Some(UserAttentionType::Critical));
        }
    }
    if let Some(thread_id) = record.thread_id {
        *shell.last_banner.lock().unwrap() = Some((thread_id, std::time::Instant::now()));
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![station_call, shell_action])
        .setup(|app| {
            let cfg = station_core::config_from_env()?;
            let engine = tauri::async_runtime::block_on(Engine::start(cfg))?;
            let shell = Arc::new(Shell { engine: engine.clone(), last_banner: Mutex::new(None) });
            app.manage(shell.clone());

            let handle = app.handle().clone();
            TrayIconBuilder::with_id("station")
                .icon(app.default_window_icon().cloned().expect("bundle icon"))
                .icon_as_template(false)
                .show_menu_on_left_click(true)
                .on_menu_event({
                    let shell = shell.clone();
                    move |app, event| {
                        let id = event.id().as_ref().to_string();
                        if let Some(thread) = id.strip_prefix("thread:").and_then(|t| t.parse::<i64>().ok()) {
                            show_main(app);
                            let _ = app.emit("station-open", serde_json::json!({ "thread_id": thread }));
                            return;
                        }
                        match id.as_str() {
                            "open" => show_main(app),
                            "quit" => {
                                shell.engine.shutdown();
                                app.exit(0);
                            }
                            "focus-blocked" | "focus-off" => {
                                let engine = shell.engine.clone();
                                let args = if id == "focus-blocked" {
                                    serde_json::json!({ "mode": "blocked-only", "minutes": 60 })
                                } else {
                                    serde_json::json!({ "mode": "off" })
                                };
                                tauri::async_runtime::spawn(async move {
                                    let _ = engine.call("set_focus", args).await;
                                });
                            }
                            _ => {}
                        }
                    }
                })
                .build(app)?;

            // Engine events -> window, menu bar, dock, notifications.
            let mut events = engine.subscribe();
            let pump_shell = shell.clone();
            tauri::async_runtime::spawn(async move {
                let _ = rebuild_tray(&handle, &pump_shell.engine.snapshot());
                loop {
                    match events.recv().await {
                        Ok(event) => {
                            let _ = handle.emit("station-event", &event);
                            match &event {
                                StationEvent::Notify { record } => deliver(&handle, &pump_shell, record),
                                StationEvent::Changed { .. } => {
                                    // Collapse bursts: rebuild the menu once things settle.
                                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                                    while events.try_recv().map(|e| {
                                        let _ = handle.emit("station-event", &e);
                                        if let StationEvent::Notify { record } = &e {
                                            deliver(&handle, &pump_shell, record);
                                        }
                                    }).is_ok() {}
                                    let _ = rebuild_tray(&handle, &pump_shell.engine.snapshot());
                                }
                                StationEvent::Badge { .. } => {}
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            let _ = handle.emit("station-event", &StationEvent::Changed { rev: 0 });
                        }
                        Err(_) => break,
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // Closing the window keeps attending from the menu bar.
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            WindowEvent::Focused(true) => {
                let shell = window.state::<Arc<Shell>>();
                let recent = shell.last_banner.lock().unwrap().take();
                if let Some((thread_id, at)) = recent {
                    if at.elapsed() < std::time::Duration::from_secs(90) {
                        let _ = window.emit("station-open", serde_json::json!({ "thread_id": thread_id }));
                    }
                }
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running Telex Station");
}
