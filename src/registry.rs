//! The local holder registry: a small on-disk record of which telex address each live local
//! station ("holder") serves, so `send`/`reply` can default the message `from` to the lease this
//! session actually holds instead of forcing `--from` / `$TELEX_ADDRESS`.
//!
//! Why a registry at all: the holder (`attach`) and `send` are separate processes; the holder
//! writes no other local record of its address, and the only local artifact (the IPC endpoint) is
//! named by a *lossy* `ipc::sanitize()` that can't be reverse-mapped. The backend lease row has no
//! reverse index from "this session" to "the address it holds." So the holder publishes a tiny
//! JSON record under `run_dir()/holders/`, and resolution reads it back.
//!
//! Liveness is decided by an `ipc::ping` to the holder's endpoint (see `live_local_holders`), not by
//! trusting file presence: a record left behind by a hard-killed holder is simply ignored because
//! its endpoint no longer answers. Records are scoped to `(backend, host)` so a holder on one
//! backend is never inferred for a send on a different backend (a real cross-backend foot-gun).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::config;
use crate::ipc;

/// One live local station's published record. The file name encodes `(sanitized-address, pid)`;
/// the authoritative address is the `address` field (the file name is lossy and for humans).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HolderRecord {
    /// The real (un-sanitized) address this station serves.
    pub address: String,
    /// Backend scope key (`profile.target()`), so resolution only considers same-store holders.
    pub backend: String,
    /// Host that wrote the record; resolution ignores other hosts (a shared `$TELEX_HOME`).
    pub host: String,
    /// Holder process id (also disambiguates the file name).
    pub pid: i64,
    /// The IPC endpoint name (named pipe / socket path), for debugging. Not load-bearing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<String>,
    /// When the holder published the record.
    pub started_at_ms: i64,
}

/// `run_dir()/holders` — created on demand by `write`.
pub fn holders_dir() -> Result<PathBuf> {
    Ok(config::run_dir()?.join("holders"))
}

fn file_name(address: &str, pid: i64) -> String {
    format!("{}-{}.json", ipc::sanitize(address), pid)
}

/// Publish (or replace) this holder's record. Writes atomically (temp file + rename) so a
/// concurrent `list()` never observes a half-written file.
pub fn write(record: &HolderRecord) -> Result<()> {
    write_in(&holders_dir()?, record)
}

/// `write` against an explicit directory (test seam — avoids mutating `$TELEX_HOME`).
pub fn write_in(dir: &Path, record: &HolderRecord) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let name = file_name(&record.address, record.pid);
    let final_path = dir.join(&name);
    let tmp_path = dir.join(format!("{name}.tmp"));
    let json = serde_json::to_string_pretty(record)?;
    std::fs::write(&tmp_path, json)?;
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

/// Best-effort removal of this holder's own record (clean-exit path).
pub fn remove(address: &str, pid: i64) {
    if let Ok(dir) = holders_dir() {
        let _ = std::fs::remove_file(dir.join(file_name(address, pid)));
    }
}

/// All parseable records currently on disk (unparseable / partial files are skipped).
pub fn list() -> Vec<HolderRecord> {
    holders_dir().map(|d| list_in(&d)).unwrap_or_default()
}

/// `list` against an explicit directory (test seam).
pub fn list_in(dir: &Path) -> Vec<HolderRecord> {
    let mut out = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(rec) = serde_json::from_str::<HolderRecord>(&text) {
                out.push(rec);
            }
        }
    }
    out
}

/// Remove every record claiming `(address, backend)`. Safe to call right after claiming the
/// exclusive lease for `address` on `backend`: lease exclusivity guarantees no *other* live holder
/// serves it, so any such file is stale.
pub fn prune_address(address: &str, backend: &str) {
    if let Ok(dir) = holders_dir() {
        prune_address_in(&dir, address, backend);
    }
}

/// `prune_address` against an explicit directory (test seam).
pub fn prune_address_in(dir: &Path, address: &str, backend: &str) {
    for rec in list_in(dir) {
        if rec.address == address && rec.backend == backend {
            let _ = std::fs::remove_file(dir.join(file_name(&rec.address, rec.pid)));
        }
    }
}

/// Unique candidate addresses from `records` that belong to this `host`+`backend`, in first-seen
/// order. Pure (no IPC) so the dedup/filter is unit-testable without a live holder.
pub fn candidate_addresses(records: &[HolderRecord], backend: &str, host: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for rec in records {
        if rec.backend == backend && rec.host == host && !out.iter().any(|a| a == &rec.address) {
            out.push(rec.address.clone());
        }
    }
    out
}

/// Addresses of *live* local holders on `backend` for this host: registry candidates whose endpoint
/// answers an `ipc::ping` for this backend. Stale records (dead/hung holder, or a holder that now
/// serves the same address on a *different* store) are dropped here.
pub async fn live_local_holders(backend: &str) -> Vec<String> {
    let host = config::hostname();
    let candidates = candidate_addresses(&list(), backend, &host);
    let mut live = Vec::new();
    for addr in candidates {
        if ipc::ping(&addr, backend).await {
            live.push(addr);
        }
    }
    live
}

/// Whether a live local holder on `backend` (this host) currently serves `address`. Used for the
/// soft-warn on an explicit/`$TELEX_ADDRESS` `from` that this host isn't actually serving. The
/// registry gate makes it backend-scoped; the ping confirms liveness *and* same-backend identity.
pub async fn is_served_locally(address: &str, backend: &str) -> bool {
    let host = config::hostname();
    let has_record = list()
        .iter()
        .any(|r| r.address == address && r.backend == backend && r.host == host);
    has_record && ipc::ping(address, backend).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::test_support::{env_guard, spawn_pong_holder};
    use crate::model::now_ms;

    fn rec(address: &str, backend: &str, host: &str, pid: i64) -> HolderRecord {
        HolderRecord {
            address: address.to_string(),
            backend: backend.to_string(),
            host: host.to_string(),
            pid,
            socket: None,
            started_at_ms: now_ms(),
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "telex-reg-{tag}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_list_remove_roundtrip() {
        let dir = temp_dir("rt");
        let r = rec("impl:a", "store-1", "h1", 100);
        write_in(&dir, &r).unwrap();
        let listed = list_in(&dir);
        assert_eq!(listed, vec![r.clone()]);
        let _ = std::fs::remove_file(dir.join(file_name(&r.address, r.pid)));
        assert!(list_in(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn distinct_addresses_that_sanitize_alike_do_not_collide() {
        // "a:b" and "a-b" both sanitize to "a-b"; keying the file by (sanitized, pid) keeps them
        // separate, and the authoritative address lives in the file content.
        let dir = temp_dir("collide");
        write_in(&dir, &rec("a:b", "store-1", "h1", 1)).unwrap();
        write_in(&dir, &rec("a_b", "store-1", "h1", 2)).unwrap();
        let mut addrs: Vec<String> = list_in(&dir).into_iter().map(|r| r.address).collect();
        addrs.sort();
        assert_eq!(addrs, vec!["a:b".to_string(), "a_b".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_only_matching_address_and_backend() {
        let dir = temp_dir("prune");
        write_in(&dir, &rec("impl:a", "store-1", "h1", 1)).unwrap();
        write_in(&dir, &rec("impl:a", "store-2", "h1", 2)).unwrap(); // same addr, other backend
        write_in(&dir, &rec("impl:b", "store-1", "h1", 3)).unwrap(); // other addr, same backend
        prune_address_in(&dir, "impl:a", "store-1");
        let mut remaining: Vec<(String, String)> = list_in(&dir)
            .into_iter()
            .map(|r| (r.address, r.backend))
            .collect();
        remaining.sort();
        assert_eq!(
            remaining,
            vec![
                ("impl:a".to_string(), "store-2".to_string()),
                ("impl:b".to_string(), "store-1".to_string()),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn candidate_addresses_dedups_and_filters_by_host_and_backend() {
        let recs = vec![
            rec("impl:a", "store-1", "h1", 1),
            rec("impl:a", "store-1", "h1", 9), // dup address (different pid) → once
            rec("impl:b", "store-1", "h1", 2),
            rec("impl:c", "store-2", "h1", 3), // other backend → excluded
            rec("impl:d", "store-1", "h2", 4), // other host → excluded
        ];
        assert_eq!(
            candidate_addresses(&recs, "store-1", "h1"),
            vec!["impl:a".to_string(), "impl:b".to_string()]
        );
    }

    // Integration: real registry file + real IPC ping. Mutates $TELEX_HOME (Unix endpoint paths),
    // so it holds the shared ENV_LOCK and restores the var afterward. The lock is intentionally held
    // across awaits to serialize all $TELEX_HOME access for the duration of the test.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn live_local_holders_includes_live_and_skips_stale() {
        let _guard = env_guard();
        let home =
            std::env::temp_dir().join(format!("telex-home-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(&home).unwrap();
        let prev = std::env::var_os("TELEX_HOME");
        std::env::set_var("TELEX_HOME", &home);

        let backend = "store-1";
        let host = config::hostname();
        let live_addr = format!("test:live:{}-{}", std::process::id(), now_ms());
        let dead_addr = format!("test:dead:{}-{}", std::process::id(), now_ms());

        // A live holder for live_addr; a stale record for dead_addr with no holder.
        let holder = spawn_pong_holder(&live_addr, backend);
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        write(&rec(&live_addr, backend, &host, 1)).unwrap();
        write(&rec(&dead_addr, backend, &host, 2)).unwrap();

        let live = live_local_holders(backend).await;
        assert!(live.contains(&live_addr), "live holder must be included");
        assert!(!live.contains(&dead_addr), "stale record must be skipped");

        // Backend scoping: a different backend sees neither.
        assert!(live_local_holders("other-store").await.is_empty());

        // served-locally is backend-scoped and liveness-checked.
        assert!(is_served_locally(&live_addr, backend).await);
        assert!(!is_served_locally(&live_addr, "other-store").await);
        assert!(!is_served_locally(&dead_addr, backend).await);

        // Cross-backend stale record: a record for live_addr on a *different* store must NOT be
        // treated as live just because live_addr's endpoint (serving `backend`) answers — the ping
        // checks served_backend, so the other-store record is correctly skipped.
        write(&rec(&live_addr, "store-2", &host, 3)).unwrap();
        assert!(
            live_local_holders("store-2").await.is_empty(),
            "a same-address holder on another store must not satisfy store-2 inference"
        );

        holder.abort();
        match prev {
            Some(v) => std::env::set_var("TELEX_HOME", v),
            None => std::env::remove_var("TELEX_HOME"),
        }
        let _ = std::fs::remove_dir_all(&home);
    }
}
