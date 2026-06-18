//! Backend profiles: named, persisted backend configurations in `~/.telex/config.toml`.
//!
//! A "backend" is a named profile (a key) describing one configured store: its kind
//! (sqlite/postgres), its connection, and — for postgres — how to authenticate. At
//! runtime a backend is selected by name (`--backend <key>` / `$TELEX_BACKEND` / the
//! `default` pointer), falling back to an implicit `default` SQLite store so a fresh
//! machine works with zero setup.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::backend::Backend;

/// The on-disk `config.toml`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ConfigFile {
    /// Name of the backend used when none is selected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default)]
    pub backends: BTreeMap<String, BackendProfile>,
}

/// One named backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendProfile {
    /// "sqlite" | "postgres".
    pub kind: String,

    // --- sqlite ---
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    // --- postgres ---
    /// Connection string: a libpq URI or a key=value DSN.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// "password" | "entra" (entra requires the `entra` feature).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<String>,
    /// Read the password from this environment variable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_env: Option<String>,
    /// Obtain the password by running this shell command (stdout, trimmed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_command: Option<String>,
    /// Optional schema to isolate telex tables in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Entra credential mode: "auto" (dev/CLI login), "cli", or "managed".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entra_cred: Option<String>,
    /// Override the Entra token scope (defaults to the Azure PG Flexible Server scope).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entra_scope: Option<String>,
}

impl BackendProfile {
    /// A short human description of where this backend points.
    pub fn target(&self) -> String {
        match self.kind.as_str() {
            "sqlite" => self.path.clone().unwrap_or_else(default_sqlite_path),
            "postgres" => self
                .url
                .as_deref()
                .map(redact_conn)
                .unwrap_or_else(|| "(no connection string)".into()),
            _ => "(unknown)".into(),
        }
    }
}

/// A stable key identifying the *effective* physical store this profile resolves to — including the
/// inputs that change which store is actually opened: the sqlite `--db` override + `~` expansion
/// (mirroring `build`), and the postgres `schema` (the telex-table isolation boundary). The holder
/// registry is scoped by this key so a station on one store is never inferred as the `from` for a
/// send on another (DECISIONS 0010). Unlike `target()` (a display string) it must distinguish
/// same-server/different-schema and same-profile/different-`--db` stores.
pub fn store_key(profile: &BackendProfile, db_override: Option<&str>) -> String {
    match profile.kind.as_str() {
        "sqlite" => {
            let path = db_override
                .map(str::to_string)
                .or_else(|| profile.path.clone())
                .map(|p| expand_tilde(&p))
                .unwrap_or_else(default_sqlite_path);
            format!("sqlite:{path}")
        }
        "postgres" => format!(
            "postgres:{}|{}",
            profile.url.as_deref().map(redact_conn).unwrap_or_default(),
            profile.schema.as_deref().unwrap_or("")
        ),
        other => format!("{other}:{}", profile.target()),
    }
}

pub fn config_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("TELEX_CONFIG") {
        return Ok(PathBuf::from(p));
    }
    Ok(crate::config::telex_home()?.join("config.toml"))
}

pub fn load() -> Result<ConfigFile> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(ConfigFile::default());
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

pub fn save(cfg: &ConfigFile) -> Result<()> {
    crate::config::ensure_home()?;
    let path = config_path()?;
    let text = toml::to_string_pretty(cfg)?;
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Resolve which backend to use: explicit selector, else `$TELEX_BACKEND`, else the
/// config `default` pointer, else an implicit `default` SQLite store.
pub fn resolve(
    selector: Option<&str>,
    db_override: Option<&str>,
) -> Result<(String, BackendProfile)> {
    let cfg = load()?;
    let name = selector
        .map(str::to_string)
        .or_else(|| std::env::var("TELEX_BACKEND").ok())
        .or_else(|| cfg.default.clone());

    match name {
        Some(n) => {
            let p = cfg.backends.get(&n).cloned().ok_or_else(|| {
                anyhow!(
                    "no backend named '{n}'. Add it with `telex backend add {n} ...`, \
                     or run `telex backend list`."
                )
            })?;
            Ok((n, p))
        }
        None => Ok(("default".to_string(), implicit_sqlite(db_override))),
    }
}

/// The built-in zero-config SQLite backend used when nothing is configured.
pub fn implicit_sqlite(db_override: Option<&str>) -> BackendProfile {
    BackendProfile {
        kind: "sqlite".into(),
        path: db_override.map(str::to_string),
        url: None,
        auth: None,
        password_env: None,
        password_command: None,
        schema: None,
        entra_cred: None,
        entra_scope: None,
    }
}

/// Build a live backend from a profile (initializing its schema).
#[allow(unused_variables)]
pub async fn build(
    profile: &BackendProfile,
    db_override: Option<&str>,
) -> Result<Arc<dyn Backend>> {
    match profile.kind.as_str() {
        #[cfg(feature = "sqlite")]
        "sqlite" => {
            let path = db_override
                .map(str::to_string)
                .or_else(|| profile.path.clone())
                .map(|p| expand_tilde(&p))
                .unwrap_or_else(default_sqlite_path);
            let b = crate::backend::sqlite::SqliteBackend::open(&path)?;
            b.init_schema().await?;
            Ok(Arc::new(b))
        }
        #[cfg(feature = "postgres")]
        "postgres" => {
            let (cfg, schema) = pg_connect_config(profile).await?;
            let b =
                crate::backend::postgres::PgBackend::connect_with(cfg, schema.as_deref()).await?;
            b.init_schema().await?;
            Ok(Arc::new(b))
        }
        other => bail!(
            "backend kind '{other}' is not available in this build of telex. \
             Reinstall with `cargo install telex --features {other}`."
        ),
    }
}

/// Build the `tokio_postgres::Config` (with resolved password) plus optional schema
/// for a postgres profile. Used both to build the backend and to open the holder's
/// LISTEN connection for push.
#[cfg(feature = "postgres")]
pub async fn pg_connect_config(
    profile: &BackendProfile,
) -> Result<(tokio_postgres::Config, Option<String>)> {
    let url = profile
        .url
        .as_deref()
        .ok_or_else(|| anyhow!("postgres backend has no connection string"))?;
    let mut cfg: tokio_postgres::Config = url
        .parse()
        .context("parsing postgres connection string (expected a libpq URI or key=value DSN)")?;

    let auth = profile.auth.as_deref().unwrap_or("password");
    match auth {
        "password" => {
            if let Some(secret) = resolve_password(profile).await? {
                cfg.password(secret);
            } else if cfg.get_password().is_none() {
                bail!(
                    "postgres backend uses password auth but no password is configured \
                     (set --password-env, --password-command, or include it in the connection string)"
                );
            }
        }
        "entra" => {
            #[cfg(feature = "entra")]
            {
                let scope = profile
                    .entra_scope
                    .as_deref()
                    .unwrap_or(crate::credential::DEFAULT_ENTRA_SCOPE);
                let mode = profile.entra_cred.as_deref().unwrap_or("auto");
                let token = crate::credential::entra_token(scope, mode).await?;
                cfg.password(token);
            }
            #[cfg(not(feature = "entra"))]
            bail!(
                "this backend uses Entra auth, which requires a telex build with `--features entra`"
            );
        }
        other => bail!("unknown auth mode '{other}' (expected password or entra)"),
    }
    Ok((cfg, profile.schema.clone()))
}

#[cfg(feature = "postgres")]
async fn resolve_password(profile: &BackendProfile) -> Result<Option<String>> {
    if let Some(env) = &profile.password_env {
        let v =
            std::env::var(env).with_context(|| format!("reading password from env var {env}"))?;
        return Ok(Some(v));
    }
    if let Some(cmd) = &profile.password_command {
        return Ok(Some(run_command(cmd).await?));
    }
    Ok(None)
}

#[cfg(feature = "postgres")]
async fn run_command(cmd: &str) -> Result<String> {
    let output = if cfg!(windows) {
        tokio::process::Command::new("cmd")
            .arg("/C")
            .arg(cmd)
            .output()
            .await
    } else {
        tokio::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .await
    }
    .with_context(|| format!("running password_command: {cmd}"))?;
    if !output.status.success() {
        bail!(
            "password_command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn default_sqlite_path() -> String {
    crate::config::telex_home()
        .map(|h| h.join("telex.db").to_string_lossy().to_string())
        .unwrap_or_else(|_| "telex.db".into())
}

// Only the sqlite path-resolution code calls `expand_tilde`; under non-sqlite feature sets
// (e.g. `--no-default-features --features postgres`) it is unreferenced, and the CI build runs
// with `-D warnings`, which would otherwise reject it as dead code.
#[cfg_attr(not(feature = "sqlite"), allow(dead_code))]
fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    p.to_string()
}

/// Redact an embedded password from a connection string for display.
fn redact_conn(conn: &str) -> String {
    let mut out = conn.to_string();
    // key=value DSN: password=...
    if let Some(idx) = out.to_ascii_lowercase().find("password=") {
        let start = idx + "password=".len();
        let end = out[start..]
            .find(|c: char| c.is_whitespace())
            .map(|e| start + e)
            .unwrap_or(out.len());
        out.replace_range(start..end, "***");
    }
    // URI userinfo: scheme://user:pass@host
    if let (Some(scheme_end), Some(at)) = (out.find("://"), out.find('@')) {
        let userinfo = &out[scheme_end + 3..at];
        if let Some(colon) = userinfo.find(':') {
            let pass_start = scheme_end + 3 + colon + 1;
            out.replace_range(pass_start..at, "***");
        }
    }
    out
}
