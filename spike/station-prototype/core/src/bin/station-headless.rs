//! Headless Station: the real engine behind a loopback HTTP/SSE API.
//!
//!   STATION_HOME=~/.telex-station station-headless [--port 4317] [--static dist]

use station_core::engine::Engine;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut port: u16 = 4317;
    let mut static_dir: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => port = args.next().and_then(|v| v.parse().ok()).unwrap_or(port),
            "--static" => static_dir = args.next().map(PathBuf::from),
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    let cfg = station_core::config_from_env()?;
    eprintln!("station config: {cfg:?}");
    let engine = Engine::start(cfg).await?;
    station_core::server::serve(engine, port, static_dir).await
}
