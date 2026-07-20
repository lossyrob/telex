use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const DEFAULT_STATION_ADDRESS: &str = "operator:rob";
pub const DEFAULT_INGRESS_ADDRESS: &str = "attention:rob";
pub const DEFAULT_TELEX_EXECUTABLE: &str = "telex";

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub station_address: String,
    pub ingress_address: String,
    pub telex_executable: String,
    pub database_path: PathBuf,
    pub store_fingerprint: String,
    pub scope_key: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub station_address: String,
    pub ingress_address: String,
    pub telex_executable: String,
    pub store_fingerprint: String,
    pub session_id: String,
    pub courier_timeout_ms: u64,
    pub status_refresh_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedScope {
    version: u8,
    station_address: String,
    store_fingerprint: String,
    session_id: String,
    last_observed_max_message_id: i64,
}

#[derive(Debug)]
pub struct LocalScope {
    path: PathBuf,
    persisted: PersistedScope,
}

impl RuntimeConfig {
    pub fn from_env() -> Result<Self, String> {
        let database_path = env::var_os("TELEX_OPERATOR_SPIKE_DB")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| {
                "TELEX_OPERATOR_SPIKE_DB is required and must name an existing SQLite database"
                    .to_string()
            })?;
        let store_fingerprint = fingerprint_database(&database_path)?;
        let station_address = nonempty_env("TELEX_OPERATOR_SPIKE_ADDRESS")
            .unwrap_or_else(|| DEFAULT_STATION_ADDRESS.to_string());
        let ingress_address = nonempty_env("TELEX_OPERATOR_SPIKE_INGRESS")
            .unwrap_or_else(|| DEFAULT_INGRESS_ADDRESS.to_string());
        let telex_executable = nonempty_env("TELEX_OPERATOR_SPIKE_TELEX")
            .unwrap_or_else(|| DEFAULT_TELEX_EXECUTABLE.to_string());
        let scope_key = scope_key(&station_address, &store_fingerprint);
        Ok(Self {
            station_address,
            ingress_address,
            telex_executable,
            database_path,
            store_fingerprint,
            scope_key,
        })
    }

    pub fn public(&self, session_id: &str) -> AppConfig {
        AppConfig {
            station_address: self.station_address.clone(),
            ingress_address: self.ingress_address.clone(),
            telex_executable: self.telex_executable.clone(),
            store_fingerprint: self.store_fingerprint.clone(),
            session_id: session_id.to_string(),
            courier_timeout_ms: 30_000,
            status_refresh_ms: 5_000,
        }
    }

    pub fn redact(&self, value: &str) -> String {
        let raw = self.database_path.to_string_lossy();
        value.replace(raw.as_ref(), "<operator-spike-db>")
    }
}

impl LocalScope {
    pub fn load_or_create(app_data_dir: &Path, config: &RuntimeConfig) -> Result<Self, String> {
        let dir = app_data_dir.join("runtime");
        fs::create_dir_all(&dir)
            .map_err(|error| format!("creating Station app-data directory failed: {error}"))?;
        let path = dir.join(format!("{}.json", config.scope_key));
        let persisted = if path.exists() {
            let body = fs::read_to_string(&path)
                .map_err(|error| format!("reading Station local scope failed: {error}"))?;
            let parsed: PersistedScope = serde_json::from_str(&body)
                .map_err(|error| format!("parsing Station local scope failed: {error}"))?;
            if parsed.version != 1
                || parsed.station_address != config.station_address
                || parsed.store_fingerprint != config.store_fingerprint
                || Uuid::parse_str(&parsed.session_id).is_err()
            {
                return Err("Station local scope does not match its safe address/store key".into());
            }
            parsed
        } else {
            PersistedScope {
                version: 1,
                station_address: config.station_address.clone(),
                store_fingerprint: config.store_fingerprint.clone(),
                session_id: Uuid::new_v4().to_string(),
                last_observed_max_message_id: 0,
            }
        };
        let scope = Self { path, persisted };
        scope.save()?;
        Ok(scope)
    }

    pub fn session_id(&self) -> &str {
        &self.persisted.session_id
    }

    pub fn high_water(&self) -> i64 {
        self.persisted.last_observed_max_message_id
    }

    pub fn set_high_water(&mut self, message_id: i64) -> Result<(), String> {
        if message_id > self.persisted.last_observed_max_message_id {
            self.persisted.last_observed_max_message_id = message_id;
            self.save()?;
        }
        Ok(())
    }

    fn save(&self) -> Result<(), String> {
        let body = serde_json::to_vec_pretty(&self.persisted)
            .map_err(|error| format!("serializing Station local scope failed: {error}"))?;
        fs::write(&self.path, body)
            .map_err(|error| format!("writing Station local scope failed: {error}"))
    }
}

pub fn fingerprint_database(path: &Path) -> Result<String, String> {
    if !path.is_file() {
        return Err("TELEX_OPERATOR_SPIKE_DB must name an existing database file".into());
    }
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("canonicalizing TELEX_OPERATOR_SPIKE_DB failed: {error}"))?;
    Ok(fingerprint_normalized_path(&canonical.to_string_lossy()))
}

fn fingerprint_normalized_path(path: &str) -> String {
    let stripped = path
        .strip_prefix(r"\\?\")
        .or_else(|| path.strip_prefix("//?/"))
        .unwrap_or(path);
    let normalized = stripped.replace('\\', "/").to_ascii_lowercase();
    format!("sha256:{:x}", Sha256::digest(normalized.as_bytes()))
}

fn scope_key(address: &str, fingerprint: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("{address}\0{fingerprint}").as_bytes())
    )
}

fn nonempty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_test_dir(name: &str) -> PathBuf {
        PathBuf::from("target")
            .join("operator-station-spike-tests")
            .join(format!("{name}-{}", Uuid::new_v4()))
    }

    #[test]
    fn fingerprint_strips_prefix_normalizes_and_uses_full_sha256() {
        let first = fingerprint_normalized_path(r"\\?\C:\Users\Rob\station.sqlite");
        let second = fingerprint_normalized_path("c:/users/rob/station.sqlite");
        assert_eq!(first, second);
        assert!(first.starts_with("sha256:"));
        assert_eq!(first.len(), "sha256:".len() + 64);
    }

    #[test]
    fn fingerprint_normalization_is_deterministic_for_unicode_paths() {
        let first = fingerprint_normalized_path(r"\\?\C:\TÜRKİYE\İ.db");
        let second = fingerprint_normalized_path("c:/tÜrkİye/İ.db");
        assert_eq!(first, second);
    }

    #[test]
    fn fingerprint_requires_an_existing_database() {
        let missing = project_test_dir("missing").join("db.sqlite");
        assert!(fingerprint_database(&missing).is_err());
    }

    #[test]
    fn persisted_state_is_scoped_by_address_and_fingerprint() {
        let root = project_test_dir("scope");
        fs::create_dir_all(&root).unwrap();
        let base = RuntimeConfig {
            station_address: "operator:rob".into(),
            ingress_address: "attention:rob".into(),
            telex_executable: "telex".into(),
            database_path: PathBuf::from("not-persisted.sqlite"),
            store_fingerprint: "sha256:aaa".into(),
            scope_key: scope_key("operator:rob", "sha256:aaa"),
        };
        let mut first = LocalScope::load_or_create(&root, &base).unwrap();
        let session = first.session_id().to_string();
        first.set_high_water(42).unwrap();
        let reopened = LocalScope::load_or_create(&root, &base).unwrap();
        assert_eq!(reopened.session_id(), session);
        assert_eq!(reopened.high_water(), 42);

        let mut other = base.clone();
        other.station_address = "operator:other".into();
        other.scope_key = scope_key(&other.station_address, &other.store_fingerprint);
        let isolated = LocalScope::load_or_create(&root, &other).unwrap();
        assert_ne!(isolated.session_id(), session);
        assert_eq!(isolated.high_water(), 0);
        fs::remove_dir_all(root).ok();
    }
}
