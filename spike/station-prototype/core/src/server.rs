//! Loopback HTTP + SSE bridge so the same React UI can run in a browser against
//! the real engine (used for development and end-to-end tests). The Tauri shell
//! does not use this; it calls the engine in-process.

use crate::engine::Engine;
use crate::model::StationEvent;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::Stream;
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

#[derive(Deserialize)]
struct Call {
    cmd: String,
    #[serde(default)]
    args: Value,
}

async fn call(State(engine): State<Arc<Engine>>, Json(call): Json<Call>) -> Json<Value> {
    match engine.call(&call.cmd, call.args).await {
        Ok(result) => Json(json!({ "ok": true, "result": result })),
        Err(error) => Json(json!({ "ok": false, "error": format!("{error:#}") })),
    }
}

async fn events(State(engine): State<Arc<Engine>>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(engine.subscribe()).filter_map(|event| {
        let event: StationEvent = match event {
            Ok(event) => event,
            // A lagged subscriber simply resyncs from the next snapshot.
            Err(_) => StationEvent::Changed { rev: 0 },
        };
        serde_json::to_string(&event).ok().map(|data| Ok(Event::default().data(data)))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub fn router(engine: Arc<Engine>, static_dir: Option<PathBuf>) -> Router {
    let api = Router::new()
        .route("/api/call", post(call))
        .route("/api/events", get(events))
        .with_state(engine);
    match static_dir {
        Some(dir) => api.fallback_service(tower_http::services::ServeDir::new(dir)),
        None => api,
    }
}

pub async fn serve(engine: Arc<Engine>, port: u16, static_dir: Option<PathBuf>) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    eprintln!("station-headless listening on http://127.0.0.1:{port}");
    axum::serve(listener, router(engine, static_dir)).await?;
    Ok(())
}
