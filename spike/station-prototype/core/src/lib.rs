//! Prototype Telex Station engine. UI-independent: the Tauri shell and the
//! headless HTTP server both drive the same [`engine::Engine`].

pub mod card;
pub mod engine;
pub mod model;
pub mod org;
pub mod policy;
pub mod port;
pub mod port_cli;
pub mod port_client;
pub mod server;
pub mod view;

use engine::{PortKind, StationConfig, TelexMode};
use std::path::PathBuf;

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Resolve the prototype configuration from `STATION_*` environment variables.
///
/// By default the Station runs against an isolated Telex sandbox under
/// `~/.telex-station/sandbox` using the `telex` binary built from this
/// worktree, so it never touches the user's real `~/.telex`.
pub fn config_from_env() -> anyhow::Result<StationConfig> {
    let base = env_path("STATION_HOME").unwrap_or_else(engine::default_data_dir);
    let operator = std::env::var("STATION_OPERATOR")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "operator".into());
    let telex = if let Some(trusted_root) = env_path("STATION_INSTALLED_ROOT") {
        TelexMode::Installed { trusted_root, cli: env_path("STATION_TELEX_EXE") }
    } else {
        let executable = env_path("STATION_TELEX_EXE")
            .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../target/debug/telex")));
        let executable = std::fs::canonicalize(&executable).map_err(|e| {
            anyhow::anyhow!("telex executable {} not found ({e}); build it or set STATION_TELEX_EXE", executable.display())
        })?;
        TelexMode::Sandbox { root: env_path("STATION_SANDBOX").unwrap_or_else(|| base.join("sandbox")), executable }
    };
    Ok(StationConfig {
        data_dir: env_path("STATION_DATA").unwrap_or_else(|| base.join("data")),
        telex,
        port: match std::env::var("STATION_PORT").ok().as_deref() {
            Some("cli") => PortKind::Cli,
            Some("client") | Some("application-client") => PortKind::ApplicationClient,
            _ => PortKind::platform_default(),
        },
        backend: std::env::var("STATION_BACKEND").ok().filter(|v| !v.is_empty()),
        default_address: std::env::var("STATION_ADDRESS")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| format!("operator:{operator}")),
        operator_name: operator,
    })
}
