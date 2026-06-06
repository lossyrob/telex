//! Process-level settings (backend selector, db override, default address, liveness
//! window) and `~/.telex/` locations. The actual backend configuration lives in
//! `profiles` (config.toml); this module only carries the per-invocation selection.

use anyhow::{anyhow, Result};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    /// `--backend <name>` (or `$TELEX_BACKEND`): which configured backend to use.
    pub backend_selector: Option<String>,
    /// `--db <path>` (or `$TELEX_DB`): override the SQLite path for this invocation.
    pub db_override: Option<String>,
    pub default_address: Option<String>,
    pub liveness_window_secs: i64,
}

/// The `~/.telex` directory, created on demand.
pub fn telex_home() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("TELEX_HOME") {
        return Ok(PathBuf::from(dir));
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow!("could not determine home directory"))?;
    Ok(home.join(".telex"))
}

/// Directory for runtime artifacts (unix IPC sockets, etc.).
pub fn run_dir() -> Result<PathBuf> {
    Ok(telex_home()?.join("run"))
}

pub fn ensure_home() -> Result<PathBuf> {
    let home = telex_home()?;
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(home.join("run"))?;
    Ok(home)
}

impl Config {
    /// Carry the CLI/env selections through (clap already applies the env fallbacks).
    pub fn resolve(
        backend: Option<String>,
        db: Option<String>,
        address: Option<String>,
    ) -> Result<Self> {
        let liveness_window_secs = std::env::var("TELEX_LIVENESS_WINDOW_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(15);

        Ok(Config {
            backend_selector: backend,
            db_override: db,
            default_address: address,
            liveness_window_secs,
        })
    }

    /// Resolve the address to operate on, preferring an explicit value over the default.
    pub fn require_address(&self, explicit: &Option<String>) -> Result<String> {
        explicit
            .clone()
            .or_else(|| self.default_address.clone())
            .ok_or_else(|| anyhow!("no address given; pass --address or set $TELEX_ADDRESS"))
    }
}

pub fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".into())
}

pub fn principal() -> String {
    std::env::var("TELEX_PRINCIPAL")
        .or_else(|_| std::env::var("USERNAME"))
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".into())
}
