//! Configuration resolution: backend selection, SQLite path, default address, and
//! `~/.telex/` locations. Resolution order is CLI flag, then environment variable,
//! then a built-in default.

use anyhow::{anyhow, Result};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub backend: String,
    pub db_path: PathBuf,
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
    /// Resolve config from explicit CLI overrides plus environment/defaults.
    pub fn resolve(
        backend: Option<String>,
        db: Option<String>,
        address: Option<String>,
    ) -> Result<Self> {
        let backend = backend
            .or_else(|| std::env::var("TELEX_BACKEND").ok())
            .unwrap_or_else(|| "sqlite".to_string());

        let db_path = match db.or_else(|| std::env::var("TELEX_DB").ok()) {
            Some(p) => PathBuf::from(p),
            None => telex_home()?.join("telex.db"),
        };

        let default_address = address.or_else(|| std::env::var("TELEX_ADDRESS").ok());

        let liveness_window_secs = std::env::var("TELEX_LIVENESS_WINDOW_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(15);

        Ok(Config {
            backend,
            db_path,
            default_address,
            liveness_window_secs,
        })
    }

    pub fn db_path_str(&self) -> String {
        self.db_path.to_string_lossy().to_string()
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
