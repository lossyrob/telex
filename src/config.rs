//! Process-level settings (backend selector, db override, default address, liveness
//! window) and local Telex locations. The actual backend configuration lives in
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

/// The config root, created on demand.
pub fn telex_home() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("TELEX_HOME") {
        return Ok(PathBuf::from(dir));
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow!("could not determine home directory"))?;
    Ok(home.join(".telex"))
}

/// Directory for runtime authority artifacts (IPC sockets on Unix, cap files, etc.).
pub fn run_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(dirs::data_local_dir)
            .ok_or_else(|| anyhow!("cannot resolve LOCALAPPDATA for runtime directory"))?;
        Ok(base.join("telex").join("run"))
    }

    // On Unix the socket path is under run_dir, so this default is part of daemon rendezvous
    // compatibility and must not be "aligned" with the Windows LOCALAPPDATA default.
    #[cfg(not(windows))]
    Ok(telex_home()?.join("run"))
}

pub fn ensure_home() -> Result<PathBuf> {
    let home = telex_home()?;
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(run_dir()?)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[cfg(windows)]
    #[test]
    fn windows_run_dir_defaults_to_local_app_data_not_telex_home() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prior_home = std::env::var_os("TELEX_HOME");
        let prior_local = std::env::var_os("LOCALAPPDATA");
        let local = std::env::temp_dir().join(format!("telex-local-{}", std::process::id()));
        let home = std::env::temp_dir().join(format!("telex-home-{}", std::process::id()));

        std::env::set_var("TELEX_HOME", &home);
        std::env::set_var("LOCALAPPDATA", &local);
        let resolved = run_dir().expect("resolve run dir");

        restore_env("TELEX_HOME", prior_home);
        restore_env("LOCALAPPDATA", prior_local);

        assert_eq!(resolved, local.join("telex").join("run"));
        assert_ne!(resolved, home.join("run"));
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_run_dir_stays_under_telex_home_for_socket_rendezvous() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prior_home = std::env::var_os("TELEX_HOME");
        let home = std::env::temp_dir().join(format!("telex-home-{}", std::process::id()));

        std::env::set_var("TELEX_HOME", &home);
        let resolved = run_dir().expect("resolve run dir");

        restore_env("TELEX_HOME", prior_home);

        assert_eq!(resolved, home.join("run"));
    }

    fn restore_env(key: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}
