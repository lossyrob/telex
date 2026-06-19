//! `telex session-end`: detach the stations a Copilot CLI session owns when the session ends.
//!
//! Driven by the plugin's `sessionEnd` hook, which pipes the CLI's end-of-session JSON payload
//! (carrying `sessionId`) to this command's stdin. We look up that session's stations in the
//! ownership registry (written by `telex attach`) and stop each holder over local IPC, removing
//! a record only once its holder is confirmed stopped/gone.
//!
//! This lives in the binary (not the shell hooks) on purpose: the path/registry contract has a
//! single source of truth ([`crate::session_registry`]), there is no shell JSON parsing or env
//! round-tripping, and it is backend-independent — stopping a holder is an address-keyed IPC
//! shutdown, so this works even when the configured backend is unavailable (e.g. Entra offline).
//! Hooks must never fail noisily, so every path logs and returns exit 0.

use std::io::Read;
use std::time::Duration;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::cli::{Ctx, SessionEndArgs};
use crate::ipc::{self, Request};
use crate::session_registry;

/// Outcome of asking one holder to shut down.
enum Outcome {
    /// The holder acknowledged the shutdown request.
    Acked,
    /// No holder is listening (connect/write failed or EOF) — nothing to stop.
    Gone,
    /// A holder is listening but did not acknowledge in time — keep the record for a later retry.
    Hung,
}

pub async fn run(_ctx: &Ctx, args: SessionEndArgs) -> Result<i32> {
    // Resolve the session id: explicit flag wins, else the stdin payload's `sessionId`.
    let raw_id = args.session_id.clone().or_else(read_session_id_from_stdin);
    let session = match raw_id.and_then(|s| session_registry::pick_session_id(Some(&s), None)) {
        Some(s) => s,
        None => {
            log("session-end: no valid sessionId (flag or stdin); nothing to do");
            return Ok(0);
        }
    };
    log(&format!("session-end: sessionId={session}"));

    let dir = match session_registry::registry_dir() {
        Ok(d) => d,
        Err(e) => {
            log(&format!("session-end: cannot resolve registry dir: {e}"));
            return Ok(0);
        }
    };
    let entries = session_registry::list_station_files_in(&dir, &session).unwrap_or_default();
    log(&format!(
        "session-end: {} station(s) registered for {session}",
        entries.len()
    ));

    for (path, r) in entries {
        match shutdown_holder(&r.address).await {
            Outcome::Acked | Outcome::Gone => {
                // Stopped (or already gone). Remove just this record's file — never a blanket wipe —
                // so a sibling whose detach failed keeps its record for a later retry.
                let _ = std::fs::remove_file(&path);
                log(&format!("session-end: cleaned address={}", r.address));
            }
            Outcome::Hung => {
                log(&format!(
                    "session-end: address={} holder did not ack; keeping record for retry",
                    r.address
                ));
            }
        }
    }
    // Drop the session dir only if every record was cleaned.
    session_registry::prune_empty_session_dir(&dir, &session);
    log(&format!("session-end: done for {session}"));
    Ok(0)
}

/// Read the whole stdin payload and extract `sessionId` (or nested `data.sessionId`). Returns
/// `None` if stdin is empty/unreadable or has no session id — a normal no-op for the hook.
fn read_session_id_from_stdin() -> Option<String> {
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() || buf.trim().is_empty() {
        return None;
    }
    parse_session_id(&buf)
}

/// Pure extraction of `sessionId` from a Copilot `sessionEnd` payload, tolerant of a `data`
/// wrapper. Factored out for testing.
fn parse_session_id(payload: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    let pick = |val: &serde_json::Value| {
        val.get("sessionId")
            .and_then(|s| s.as_str())
            .map(String::from)
    };
    pick(&v).or_else(|| v.get("data").and_then(pick))
}

/// Ask the holder for `address` to shut down over local IPC, distinguishing "gone" (no holder)
/// from "hung" (holder present but unresponsive) so the caller can keep records worth retrying.
async fn shutdown_holder(address: &str) -> Outcome {
    let stream = match ipc::connect(address).await {
        Ok(s) => s,
        Err(_) => return Outcome::Gone,
    };
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut line = match serde_json::to_string(&Request::Shutdown) {
        Ok(s) => s,
        Err(_) => return Outcome::Hung,
    };
    line.push('\n');
    if write_half.write_all(line.as_bytes()).await.is_err() {
        return Outcome::Gone;
    }
    let _ = write_half.flush().await;

    let mut reader = BufReader::new(read_half);
    let mut buf = String::new();
    match tokio::time::timeout(Duration::from_secs(3), reader.read_line(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => Outcome::Acked,
        Ok(Ok(_)) => Outcome::Gone, // EOF before any frame — holder went away
        _ => Outcome::Hung,         // timeout or read error — present but unresponsive
    }
}

/// Append a line to `$TELEX_HOOK_LOG` when set, else write to stderr. Best-effort.
fn log(msg: &str) {
    if let Ok(path) = std::env::var("TELEX_HOOK_LOG") {
        if !path.trim().is_empty() {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                use std::io::Write;
                let _ = writeln!(f, "{msg}");
                return;
            }
        }
    }
    eprintln!("[session-end] {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_top_level_session_id() {
        let p = r#"{"event":"session.ended","sessionId":"abc-123","endReason":"user_exit"}"#;
        assert_eq!(parse_session_id(p).as_deref(), Some("abc-123"));
    }

    #[test]
    fn parses_nested_data_session_id() {
        let p = r#"{"data":{"sessionId":"nested-9","reason":"complete"}}"#;
        assert_eq!(parse_session_id(p).as_deref(), Some("nested-9"));
    }

    #[test]
    fn missing_or_malformed_yields_none() {
        assert_eq!(parse_session_id(""), None);
        assert_eq!(parse_session_id("not json"), None);
        assert_eq!(parse_session_id(r#"{"event":"x"}"#), None);
    }
}
