//! Host-local enrollment for externally scheduled receive checks. The daemon is the sole writer.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::daemon::DaemonPaths;
use crate::platform_fs as fs;

pub const CAPABILITY: &str = "scheduled-check-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    Prepare {
        interval_seconds: u32,
        selector: Option<String>,
    },
    Confirm {
        automation: String,
    },
    Check {
        receipt: String,
    },
    Ack {
        receipt: String,
    },
    Reply {
        receipt: String,
        body: String,
    },
    Status,
    Disable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Pending,
    Active,
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub interval_seconds: u32,
    pub confirmed_at_ms: Option<i64>,
    pub last_attempt_ms: Option<i64>,
    pub last_success_ms: Option<i64>,
    pub last_outcome: Option<String>,
}
impl Observation {
    pub fn due_ms(&self) -> Option<i64> {
        Some(
            self.last_success_ms
                .or(self.confirmed_at_ms)?
                .saturating_add(i64::from(self.interval_seconds) * 1000),
        )
    }
    pub fn overdue_ms(&self) -> Option<i64> {
        Some(
            self.due_ms()?
                .saturating_add((i64::from(self.interval_seconds) * 500).min(120_000)),
        )
    }
    pub fn health(&self, now: i64) -> crate::daemon_ipc::StationHealth {
        use crate::daemon_ipc::StationHealth::*;
        if self.confirmed_at_ms.is_none() || self.last_attempt_ms.is_some_and(|stamp| stamp > now) {
            return Unknown;
        }
        if self
            .last_success_ms
            .or(self.confirmed_at_ms)
            .is_some_and(|stamp| stamp > now)
        {
            return Unknown;
        }
        if self.overdue_ms().is_some_and(|deadline| now > deadline) {
            ScheduledOverdue
        } else if self.last_success_ms.is_some() {
            AttendedScheduled
        } else {
            AwaitingFirstCheck
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reservation {
    pub receipt: String,
    pub message_id: i64,
    pub delivery_id: i64,
    pub epoch: i64,
    pub expires_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Binding {
    version: u32,
    pub generation: String,
    pub store_key: String,
    pub logical_store_id: String,
    pub selector: Option<String>,
    pub session: String,
    pub address: String,
    pub phase: Phase,
    pub automation: Option<String>,
    pub observation: Observation,
    pub reservation: Option<Reservation>,
    pub last_ack: Option<String>,
    pub lease_epoch: Option<i64>,
}
impl Binding {
    pub fn prepare(
        store_key: String,
        logical_store_id: String,
        selector: Option<String>,
        session: String,
        address: String,
        interval_seconds: u32,
    ) -> Result<Self> {
        ensure!(
            (60..=86400).contains(&interval_seconds),
            "interval must be from one minute to one day"
        );
        Ok(Self {
            version: 1,
            generation: nonce()?,
            store_key,
            logical_store_id,
            selector,
            session,
            address,
            phase: Phase::Pending,
            automation: None,
            reservation: None,
            last_ack: None,
            lease_epoch: None,
            observation: Observation {
                interval_seconds,
                confirmed_at_ms: None,
                last_attempt_ms: None,
                last_success_ms: None,
                last_outcome: None,
            },
        })
    }
    pub fn matches(&self, store: &str, address: &str) -> bool {
        self.store_key == store && self.address == address && self.phase != Phase::Disabled
    }
    pub fn save(&self, paths: &DaemonPaths) -> Result<()> {
        let dir = directory(paths);
        fs::ensure_owner_private_dir(&dir)?;
        fs::write_owner_only_file_atomic(&file(paths, &self.session), &serde_json::to_vec(self)?)?;
        Ok(())
    }
}
pub fn nonce() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| anyhow::anyhow!("random identifier unavailable: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn directory(paths: &DaemonPaths) -> PathBuf {
    paths
        .run_dir
        .join(format!("checks-v1-{}", paths.singleton_hash))
}
fn file(paths: &DaemonPaths, session: &str) -> PathBuf {
    directory(paths).join(format!("{}.json", fs::sha256_hex(session.as_bytes())))
}
pub fn prepare_lock(paths: &DaemonPaths, session: &str) -> Result<fs::OwnerPrivateAdvisoryLock> {
    fs::ensure_owner_private_dir(&directory(paths))?;
    Ok(fs::try_acquire_owner_private_advisory_lock(
        &file(paths, session).with_extension("lock"),
    )?)
}
pub fn load(paths: &DaemonPaths, session: &str) -> Result<Option<Binding>> {
    let path = file(paths, session);
    if !fs::path_present(&path)? {
        return Ok(None);
    }
    fs::ensure_owner_private_dir(&directory(paths))?;
    let b: Binding = serde_json::from_slice(&fs::read_owner_only_file(&path, 65536)?)?;
    ensure!(
        b.version == 1 && b.session == session,
        "invalid scheduled station record"
    );
    Ok(Some(b))
}
pub fn disable(paths: &DaemonPaths, store: &str, session: &str, address: &str) -> Result<()> {
    if let Some(mut b) = load(paths, session)?.filter(|b| b.matches(store, address)) {
        b.phase = Phase::Disabled;
        b.reservation = None;
        b.save(paths)?;
    }
    Ok(())
}
pub fn sessions_for_address(
    paths: &DaemonPaths,
    store: &str,
    address: &str,
) -> Result<Vec<String>> {
    let dir = directory(paths);
    if !fs::path_present(&dir)? {
        return Ok(Vec::new());
    }
    fs::ensure_owner_private_dir(&dir)?;
    let mut sessions = Vec::new();
    for item in std::fs::read_dir(dir)? {
        let path = item?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let b: Binding = serde_json::from_slice(&fs::read_owner_only_file(&path, 65536)?)?;
        ensure!(
            b.version == 1 && path == file(paths, &b.session),
            "invalid scheduled station identity"
        );
        if b.matches(store, address) {
            sessions.push(b.session)
        }
    }
    Ok(sessions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon_ipc::StationHealth::*;
    #[test]
    fn attendance_follows_observed_success_and_clock() {
        let mut o = Observation {
            interval_seconds: 300,
            confirmed_at_ms: None,
            last_attempt_ms: None,
            last_success_ms: None,
            last_outcome: None,
        };
        assert_eq!(o.health(1000), Unknown);
        o.confirmed_at_ms = Some(1000);
        assert_eq!(o.health(1000), AwaitingFirstCheck);
        assert_eq!(o.health(422_000), ScheduledOverdue);
        o.last_attempt_ms = Some(400_000);
        o.last_success_ms = o.last_attempt_ms;
        assert_eq!(o.due_ms(), Some(700_000));
        assert_eq!(o.health(820_000), AttendedScheduled);
        o.last_attempt_ms = Some(819_000);
        o.last_outcome = Some("failed".into());
        assert_eq!(o.health(820_001), ScheduledOverdue);
        assert_eq!(o.health(100), Unknown);
    }
}
