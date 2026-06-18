//! Session→station ownership registry.
//!
//! So that a Copilot CLI `sessionEnd` hook can detach the stations a session owns when the
//! session ends (dismiss *or* quit), `telex attach` records each station it holds here, and
//! `telex detach` / the holder's clean shutdown removes it. Empirically the CLI fires
//! `sessionEnd` on both dismiss and quit, decoupled from process lifecycle — the only signal
//! that catches a dismiss, which a process-pid watch (see [`crate::session_watch`]) cannot.
//!
//! Layout — one file per station, so concurrent attaches within a single session never race on
//! a shared file:
//!
//! ```text
//! <registry_dir>/<session_id>/<sanitized-address>.json
//!   { "address": "...", "telex": "<binary path>", "env": { "TELEX_DB": "...", ... } }
//! ```
//!
//! - `registry_dir` = `$TELEX_SESSION_DIR`, else `<telex_home>/sessions`.
//! - `session_id`   = `$TELEX_SESSION_ID`, else `$COPILOT_AGENT_SESSION_ID` (validated token).
//!
//! When no session id is resolvable the registry is **disabled**: every operation is a no-op so
//! telex stays fully usable without the Copilot CLI integration. All operations are best-effort;
//! callers must never fail an attach/detach because the registry could not be written.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::ipc::sanitize;

/// Backend-selecting env vars captured at attach time, so the hook's `telex detach` resolves the
/// same store the holder used. This matters when the holder is already gone and only the lease
/// row lingers: the direct lease-release path in `detach` needs the right backend.
const CAPTURED_ENV: &[&str] = &["TELEX_HOME", "TELEX_CONFIG", "TELEX_DB", "TELEX_BACKEND"];

/// One station a session owns, as persisted in the registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationRecord {
    /// The durable address this session is holding.
    pub address: String,
    /// Path to the telex binary that owns the station, so the hook invokes the same build.
    pub telex: String,
    /// Backend-selecting env present at attach time (subset of [`CAPTURED_ENV`]).
    pub env: BTreeMap<String, String>,
}

/// Resolve the Copilot session id from the environment: explicit `$TELEX_SESSION_ID` first, then
/// the Copilot CLI's `$COPILOT_AGENT_SESSION_ID`. Returns `None` (registry disabled) when neither
/// is set or the value is not a safe filesystem token.
pub fn session_id() -> Option<String> {
    pick_session_id(
        std::env::var("TELEX_SESSION_ID").ok().as_deref(),
        std::env::var("COPILOT_AGENT_SESSION_ID").ok().as_deref(),
    )
}

/// Pure session-id precedence + validation (explicit override wins). Factored out so it is
/// testable without mutating process env.
pub fn pick_session_id(explicit: Option<&str>, copilot: Option<&str>) -> Option<String> {
    for candidate in [explicit, copilot].into_iter().flatten() {
        let v = candidate.trim();
        if !v.is_empty() && is_safe_token(v) {
            return Some(v.to_string());
        }
    }
    None
}

fn is_safe_token(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Directory holding per-session station registries.
pub fn registry_dir() -> Result<PathBuf> {
    let override_dir = std::env::var("TELEX_SESSION_DIR").ok();
    Ok(pick_registry_dir(
        override_dir.as_deref(),
        &crate::config::telex_home()?,
    ))
}

/// Pure registry-dir resolution: `$TELEX_SESSION_DIR` (when non-empty) else `<telex_home>/sessions`.
pub fn pick_registry_dir(override_dir: Option<&str>, telex_home: &Path) -> PathBuf {
    match override_dir {
        Some(d) if !d.trim().is_empty() => PathBuf::from(d),
        _ => telex_home.join("sessions"),
    }
}

/// Record that the current session owns `address`. No-op (Ok) when no session id is resolvable.
pub fn register_station(address: &str) -> Result<()> {
    let Some(session) = session_id() else {
        return Ok(());
    };
    let record = StationRecord {
        address: address.to_string(),
        telex: telex_binary(),
        env: captured_env(),
    };
    register_station_in(&registry_dir()?, &session, &record)
}

/// Remove the current session's record for `address`. No-op (Ok) when no session id is resolvable.
pub fn unregister_station(address: &str) -> Result<()> {
    let Some(session) = session_id() else {
        return Ok(());
    };
    unregister_station_in(&registry_dir()?, &session, address)
}

/// Write `record` into `<dir>/<session>/<sanitized-address>.json`. Pure of process env, for tests
/// and for callers that resolve the dir/session themselves.
pub fn register_station_in(dir: &Path, session: &str, record: &StationRecord) -> Result<()> {
    let sdir = dir.join(session);
    std::fs::create_dir_all(&sdir)?;
    let path = sdir.join(format!("{}.json", sanitize(&record.address)));
    let json = serde_json::to_string_pretty(record)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Remove `<dir>/<session>/<sanitized-address>.json`, dropping the session dir if it becomes
/// empty. Missing files are not an error.
pub fn unregister_station_in(dir: &Path, session: &str, address: &str) -> Result<()> {
    let sdir = dir.join(session);
    let path = sdir.join(format!("{}.json", sanitize(address)));
    let _ = std::fs::remove_file(&path);
    if let Ok(mut entries) = std::fs::read_dir(&sdir) {
        if entries.next().is_none() {
            let _ = std::fs::remove_dir(&sdir);
        }
    }
    Ok(())
}

/// List the stations recorded for `session` under `dir` (used by tests and tooling; the hook
/// reads the files directly in shell).
pub fn list_stations_in(dir: &Path, session: &str) -> Result<Vec<StationRecord>> {
    let sdir = dir.join(session);
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&sdir) {
        Ok(e) => e,
        Err(_) => return Ok(out),
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&p) {
            if let Ok(rec) = serde_json::from_str::<StationRecord>(&text) {
                out.push(rec);
            }
        }
    }
    out.sort_by(|a, b| a.address.cmp(&b.address));
    Ok(out)
}

fn captured_env() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for k in CAPTURED_ENV {
        if let Ok(v) = std::env::var(k) {
            env.insert((*k).to_string(), v);
        }
    }
    env
}

fn telex_binary() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(str::to_string))
        .unwrap_or_else(|| "telex".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir() -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::SeqCst);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir().join(format!(
            "telex-registry-test-{}-{}-{}",
            std::process::id(),
            now,
            seq
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn rec(address: &str) -> StationRecord {
        let mut env = BTreeMap::new();
        env.insert("TELEX_DB".to_string(), "/tmp/t.db".to_string());
        StationRecord {
            address: address.to_string(),
            telex: "telex".to_string(),
            env,
        }
    }

    #[test]
    fn session_id_precedence_and_validation() {
        // Explicit override wins over the Copilot env.
        assert_eq!(
            pick_session_id(Some("explicit-1"), Some("copilot-2")).as_deref(),
            Some("explicit-1")
        );
        // Falls back to the Copilot env when no override.
        assert_eq!(
            pick_session_id(None, Some("copilot-2")).as_deref(),
            Some("copilot-2")
        );
        // Empty / whitespace is ignored, falling through to the next candidate.
        assert_eq!(
            pick_session_id(Some("   "), Some("copilot-2")).as_deref(),
            Some("copilot-2")
        );
        // Unsafe tokens (path separators, etc.) are rejected → registry disabled.
        assert_eq!(pick_session_id(Some("a/b"), None), None);
        assert_eq!(pick_session_id(Some("../escape"), None), None);
        assert_eq!(pick_session_id(None, None), None);
    }

    #[test]
    fn registry_dir_override_else_home() {
        let home = Path::new("/home/.telex");
        assert_eq!(
            pick_registry_dir(Some("/custom/dir"), home),
            PathBuf::from("/custom/dir")
        );
        // Empty override falls back to <home>/sessions.
        assert_eq!(pick_registry_dir(Some("   "), home), home.join("sessions"));
        assert_eq!(pick_registry_dir(None, home), home.join("sessions"));
    }

    #[test]
    fn register_then_list_then_unregister() {
        let dir = temp_dir();
        let session = "sess-abc";
        assert!(list_stations_in(&dir, session).unwrap().is_empty());

        register_station_in(&dir, session, &rec("station:a")).unwrap();
        register_station_in(&dir, session, &rec("station:b")).unwrap();

        let listed = list_stations_in(&dir, session).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].address, "station:a");
        assert_eq!(listed[1].address, "station:b");
        assert_eq!(
            listed[0].env.get("TELEX_DB").map(String::as_str),
            Some("/tmp/t.db")
        );

        unregister_station_in(&dir, session, "station:a").unwrap();
        let listed = list_stations_in(&dir, session).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].address, "station:b");

        // Removing the last station drops the session directory.
        unregister_station_in(&dir, session, "station:b").unwrap();
        assert!(list_stations_in(&dir, session).unwrap().is_empty());
        assert!(!dir.join(session).exists());
    }

    #[test]
    fn register_is_idempotent_per_address() {
        let dir = temp_dir();
        let session = "sess-idem";
        register_station_in(&dir, session, &rec("station:x")).unwrap();
        register_station_in(&dir, session, &rec("station:x")).unwrap();
        assert_eq!(list_stations_in(&dir, session).unwrap().len(), 1);
    }

    #[test]
    fn unregister_missing_is_ok() {
        let dir = temp_dir();
        // No session dir at all → still Ok (no-op).
        unregister_station_in(&dir, "nope", "station:gone").unwrap();
    }
}
