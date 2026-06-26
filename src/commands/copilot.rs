//! Hidden Copilot CLI plugin adapter commands.
//!
//! This module is the harness boundary: it reads Copilot hook payloads and `COPILOT_*`
//! environment variables, then maps them to generic telex session/watch-pid inputs. Core daemon
//! protocol and identity helpers intentionally remain unaware of Copilot-specific names.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::cli::{
    AttachArgs, CopilotAttachArgs, CopilotCmd, CopilotSessionEndArgs, CopilotSkillArgs,
    CopilotTurnGuardArgs, Ctx,
};
use crate::daemon_ipc::{DaemonStatus, MemberStatus, Request, Response, WatchPidSpec};
use crate::model::now_ms;

const DEFAULT_TURN_GUARD_MAX_NUDGES: u32 = 3;
const TURN_GUARD_DISABLED: &str = "turn_guard_disabled";
const HOOK_LOG_FILE: &str = "hook-events.ndjson";

pub async fn run(ctx: &Ctx, cmd: CopilotCmd) -> Result<i32> {
    match cmd {
        CopilotCmd::Attach(args) => attach(ctx, args).await,
        CopilotCmd::SessionEnd(args) => session_end(ctx, args).await,
        CopilotCmd::TurnGuard(args) => turn_guard(ctx, args).await,
        CopilotCmd::Skill(args) => skill(args),
    }
}

async fn attach(ctx: &Ctx, args: CopilotAttachArgs) -> Result<i32> {
    let session = match resolve_copilot_session(args.session.as_deref(), None) {
        Some(session) => session,
        None => {
            eprintln!(
                "telex: no Copilot session id available; set COPILOT_AGENT_SESSION_ID or pass --session"
            );
            return Ok(1);
        }
    };
    let mut watch_pid = Vec::new();
    if let Some(pid) = copilot_loader_pid() {
        watch_pid.push(WatchPidSpec::anchor(pid));
    }
    let attach_args = AttachArgs {
        description: args.description,
        scope: args.scope,
        tags: args.tags,
        heartbeat_secs: 5,
        poll_secs: 1,
        keepalive_secs: 3,
        occupant: args.occupant,
        session: Some(session),
        push: false,
        session_pid: None,
        watch_pid,
        session_poll_secs: 2,
        no_session_bind: false,
    };
    crate::commands::attach::run(ctx, attach_args).await
}

async fn session_end(ctx: &Ctx, args: CopilotSessionEndArgs) -> Result<i32> {
    let payload = read_stdin_payload();
    let session = match resolve_copilot_session(args.session.as_deref(), payload.as_deref()) {
        Some(session) => session,
        None => {
            let event = HookLogEvent::session_end("missing_session", None, None);
            write_hook_log_best_effort(&event);
            print_json(&serde_json::json!({"session_end": false, "outcome": "missing_session"}));
            return Ok(0);
        }
    };

    let store_key = match ctx.store_key() {
        Ok(store_key) => store_key,
        Err(e) => {
            let detail = e.to_string();
            let event = HookLogEvent::session_end("store_key_error", Some(&session), Some(&detail));
            write_hook_log_best_effort(&event);
            print_json(
                &serde_json::json!({"session_end": false, "session_id": session, "outcome": "store_key_error"}),
            );
            return Ok(0);
        }
    };

    let (mut status_client, status_cap) = match connect_existing_with_cap(&store_key).await {
        Ok((client, cap)) => (client, cap),
        Err(e) => {
            let event = HookLogEvent::session_end("daemon_unavailable", Some(&session), Some(&e));
            write_hook_log_best_effort(&event);
            print_json(
                &serde_json::json!({"session_end": false, "session_id": session, "store_key": store_key, "outcome": "daemon_unavailable"}),
            );
            return Ok(0);
        }
    };

    let status = match daemon_status(&mut status_client, &store_key, &status_cap.admin_cap).await {
        Ok(status) => status,
        Err(e) => {
            let event = HookLogEvent::session_end("status_error", Some(&session), Some(&e));
            write_hook_log_best_effort(&event);
            print_json(
                &serde_json::json!({"session_end": false, "session_id": session, "store_key": store_key, "outcome": "status_error"}),
            );
            return Ok(0);
        }
    };

    let mut affected_stores = status
        .members
        .iter()
        .filter(|member| member.session_id == session && !member.idle)
        .map(|member| member.store_key.clone())
        .collect::<BTreeSet<_>>();
    if affected_stores.is_empty() {
        affected_stores.insert(store_key.clone());
    }

    let mut ended = Vec::new();
    let mut failed = Vec::new();
    for affected_store in affected_stores {
        let (mut client, cap) = match connect_existing_with_cap(&affected_store).await {
            Ok(connection) => connection,
            Err(e) => {
                failed.push(format!("{affected_store}: {e}"));
                continue;
            }
        };
        let response = client
            .request(&Request::SessionEnd {
                store_key: affected_store.clone(),
                session_id: session.clone(),
                proof: Some(cap.admin_cap),
            })
            .await;
        match response {
            Ok(Response::Ack { .. }) => ended.push(affected_store),
            Ok(Response::Error { code, message, .. }) => {
                failed.push(format!("{affected_store}: {code}: {message}"));
            }
            Ok(other) => failed.push(format!("{affected_store}: unexpected {other:?}")),
            Err(e) => failed.push(format!("{affected_store}: {e}")),
        }
    }

    cleanup_turn_guard_state_best_effort(&session);
    let outcome = if failed.is_empty() {
        "session_end"
    } else {
        "partial_session_end"
    };
    let event =
        HookLogEvent::session_end(outcome, Some(&session), failed.first().map(String::as_str));
    write_hook_log_best_effort(&event);
    print_json(&serde_json::json!({
        "session_end": failed.is_empty(),
        "session_id": session,
        "stores": ended,
        "failures": failed,
        "outcome": outcome,
    }));
    Ok(0)
}

async fn turn_guard(ctx: &Ctx, args: CopilotTurnGuardArgs) -> Result<i32> {
    let payload = read_stdin_payload();
    let session = match resolve_copilot_session(args.session.as_deref(), payload.as_deref()) {
        Some(session) => session,
        None => {
            return allow_with_log(
                None,
                "missing_session",
                "No Copilot session id was available.",
            )
        }
    };

    let settings = match GuardSettings::from_env() {
        Ok(settings) => settings,
        Err(warning) => return allow_with_log(Some(&session), "invalid_config", &warning),
    };
    if !settings.enabled {
        return allow_with_log(
            Some(&session),
            TURN_GUARD_DISABLED,
            "TELEX_TURN_GUARD disabled the guard.",
        );
    }

    let store_key = match ctx.store_key() {
        Ok(store_key) => store_key,
        Err(e) => return allow_with_log(Some(&session), "store_key_error", &e.to_string()),
    };
    let (mut client, cap) = match connect_existing_with_cap(&store_key).await {
        Ok((client, cap)) => (client, cap),
        Err(e) => return allow_with_log(Some(&session), "daemon_unavailable", &e),
    };
    let status = match daemon_status(&mut client, &store_key, &cap.admin_cap).await {
        Ok(status) => status,
        Err(e) => return allow_with_log(Some(&session), "status_error", &e),
    };

    let active_members = active_session_members(&status, &session);
    let scope_key = guard_scope_key(&store_key, &active_members);
    let state_path = turn_guard_state_path(&scope_key, &session)?;
    let _lock = match StateLock::acquire(&state_path) {
        Ok(lock) => lock,
        Err(e) => {
            return allow_with_log(
                Some(&session),
                "state_lock_error",
                &format!("could not acquire turn-guard state lock: {e}"),
            )
        }
    };
    let state = match read_guard_state(&state_path) {
        Ok(state) => state,
        Err(e) => {
            return allow_with_log(
                Some(&session),
                "state_read_error",
                &format!("could not read turn-guard state: {e}"),
            )
        }
    };
    let decision = evaluate_guard(&session, &active_members, settings, state);
    if let Some(next_state) = &decision.next_state {
        if let Err(e) = write_guard_state(&state_path, next_state) {
            return allow_with_log(
                Some(&session),
                "state_write_error",
                &format!("could not write turn-guard state: {e}"),
            );
        }
    } else {
        let _ = std::fs::remove_file(&state_path);
    }

    write_hook_log_best_effort(&HookLogEvent::turn_guard(
        decision.reason_code,
        Some(&session),
        Some(decision.summary.as_str()),
        decision.nudges,
        settings.max_nudges,
    ));
    print_json(&decision.output_json());
    Ok(0)
}

fn skill(args: CopilotSkillArgs) -> Result<i32> {
    if args.raw {
        print!("{}", crate::commands::skill::raw_skill());
    } else {
        print!("{}", crate::commands::skill::raw_skill());
    }
    Ok(0)
}

fn read_stdin_payload() -> Option<String> {
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_ok() && !buf.trim().is_empty() {
        Some(buf)
    } else {
        None
    }
}

fn resolve_copilot_session(explicit: Option<&str>, payload: Option<&str>) -> Option<String> {
    explicit
        .and_then(nonempty)
        .or_else(|| payload.and_then(parse_session_id))
        .or_else(|| env_nonempty("COPILOT_AGENT_SESSION_ID"))
}

fn parse_session_id(payload: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    json_string(&v, "sessionId")
        .or_else(|| json_string(&v, "session_id"))
        .or_else(|| v.get("data").and_then(|d| json_string(d, "sessionId")))
        .or_else(|| v.get("data").and_then(|d| json_string(d, "session_id")))
}

fn json_string(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key).and_then(|s| s.as_str()).and_then(nonempty)
}

fn nonempty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(|s| nonempty(&s))
}

fn copilot_loader_pid() -> Option<u32> {
    env_nonempty("COPILOT_LOADER_PID").and_then(|s| s.parse::<u32>().ok())
}

async fn connect_existing_with_cap(
    store_key: &str,
) -> std::result::Result<(crate::daemon::DaemonClient, crate::daemon::CapFile), String> {
    let paths = crate::daemon::DaemonPaths::current().map_err(|e| e.to_string())?;
    let cap = crate::daemon::read_cap_file(&paths.cap_path).map_err(|e| e.to_string())?;
    let client = crate::daemon::connect_existing(store_key)
        .await
        .map_err(|e| e.to_string())?;
    Ok((client, cap))
}

async fn daemon_status(
    client: &mut crate::daemon::DaemonClient,
    store_key: &str,
    admin_cap: &str,
) -> std::result::Result<DaemonStatus, String> {
    match client
        .request(&Request::Status {
            store_key: Some(store_key.to_string()),
            detail: true,
            proof: Some(admin_cap.to_string()),
        })
        .await
        .map_err(|e| e.to_string())?
    {
        Response::StatusReport { status } => Ok(status),
        Response::Error { code, message, .. } => Err(format!("{code}: {message}")),
        other => Err(format!("unexpected status response: {other:?}")),
    }
}

fn active_session_members(status: &DaemonStatus, session: &str) -> Vec<MemberStatus> {
    status
        .members
        .iter()
        .filter(|member| member.session_id == session && !member.idle)
        .cloned()
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GuardSettings {
    enabled: bool,
    max_nudges: u32,
}

impl GuardSettings {
    fn from_env() -> std::result::Result<Self, String> {
        let enabled = match env_nonempty("TELEX_TURN_GUARD") {
            Some(value) if matches!(value.to_ascii_lowercase().as_str(), "off" | "0" | "false") => {
                false
            }
            _ => true,
        };
        if !enabled {
            return Ok(Self {
                enabled,
                max_nudges: DEFAULT_TURN_GUARD_MAX_NUDGES,
            });
        }
        let max_nudges = match env_nonempty("TELEX_TURN_GUARD_MAX_NUDGES") {
            Some(value) => value.parse::<u32>().map_err(|_| {
                format!("invalid TELEX_TURN_GUARD_MAX_NUDGES={value:?}; expected unsigned integer")
            })?,
            None => DEFAULT_TURN_GUARD_MAX_NUDGES,
        };
        Ok(Self {
            enabled,
            max_nudges,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct GuardState {
    nudges: u32,
    last_decision: String,
    updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GuardEvaluation {
    decision: HookDecision,
    reason_code: &'static str,
    summary: String,
    nudges: u32,
    next_state: Option<GuardState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HookDecision {
    Allow,
    Block { reason: String },
}

impl GuardEvaluation {
    fn output_json(&self) -> serde_json::Value {
        match &self.decision {
            HookDecision::Allow => serde_json::json!({"decision": "allow"}),
            HookDecision::Block { reason } => {
                serde_json::json!({"decision": "block", "reason": reason})
            }
        }
    }
}

fn evaluate_guard(
    session: &str,
    members: &[MemberStatus],
    settings: GuardSettings,
    prior_state: Option<GuardState>,
) -> GuardEvaluation {
    if members.is_empty() {
        return GuardEvaluation {
            decision: HookDecision::Allow,
            reason_code: "no_attended_stations",
            summary: "No attended stations for this session.".to_string(),
            nudges: 0,
            next_state: None,
        };
    }

    let unarmed = members
        .iter()
        .filter(|member| member.live_waiters_count == 0)
        .collect::<Vec<_>>();
    let any_live_waiter = members.iter().any(|member| member.live_waiters_count > 0);
    if unarmed.is_empty() {
        return GuardEvaluation {
            decision: HookDecision::Allow,
            reason_code: "all_armed",
            summary: "All attended stations have live waiters.".to_string(),
            nudges: 0,
            next_state: None,
        };
    }

    let prior_nudges = if any_live_waiter {
        0
    } else {
        prior_state.map(|s| s.nudges).unwrap_or(0)
    };
    if prior_nudges >= settings.max_nudges {
        return GuardEvaluation {
            decision: HookDecision::Allow,
            reason_code: "cap_exhausted",
            summary: format!(
                "Turn guard cap exhausted after {prior_nudges} nudge(s); allowing this turn."
            ),
            nudges: prior_nudges,
            next_state: Some(GuardState {
                nudges: prior_nudges,
                last_decision: "cap_exhausted".to_string(),
                updated_at_ms: now_ms(),
            }),
        };
    }

    let nudges = prior_nudges.saturating_add(1);
    let station_list = unarmed
        .iter()
        .map(|member| {
            format!(
                "{} (pending {})",
                member.address, member.pending_unconsumed_count
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let reason = format!(
        "Telex turn guard: session {session} is attending station(s) with no live waiter: {station_list}. Re-arm `telex wait ... --out-dir <dir>` if still attending, or run `telex detach --address <station>` if done. Nudge {nudges}/{}.",
        settings.max_nudges
    );
    GuardEvaluation {
        decision: HookDecision::Block { reason },
        reason_code: "unarmed_attended_station",
        summary: station_list,
        nudges,
        next_state: Some(GuardState {
            nudges,
            last_decision: "unarmed_attended_station".to_string(),
            updated_at_ms: now_ms(),
        }),
    }
}

fn allow_with_log(session: Option<&str>, reason_code: &'static str, detail: &str) -> Result<i32> {
    write_hook_log_best_effort(&HookLogEvent::turn_guard(
        reason_code,
        session,
        Some(detail),
        0,
        DEFAULT_TURN_GUARD_MAX_NUDGES,
    ));
    print_json(&serde_json::json!({"decision": "allow"}));
    Ok(0)
}

fn guard_scope_key(default_store_key: &str, members: &[MemberStatus]) -> String {
    let stores = members
        .iter()
        .map(|member| member.store_key.clone())
        .collect::<BTreeSet<_>>();
    if stores.is_empty() {
        default_store_key.to_string()
    } else {
        stores.into_iter().collect::<Vec<_>>().join("\n")
    }
}

fn turn_guard_state_path(store_key: &str, session: &str) -> Result<PathBuf> {
    let paths = crate::daemon::DaemonPaths::current()?;
    Ok(paths
        .run_dir
        .join("copilot")
        .join("turn-guard")
        .join(crate::daemon::short_hash(store_key.as_bytes()))
        .join(format!("{}.json", path_token(session))))
}

fn hook_log_path() -> Result<PathBuf> {
    let paths = crate::daemon::DaemonPaths::current()?;
    Ok(paths.run_dir.join("copilot").join(HOOK_LOG_FILE))
}

fn path_token(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        value.to_string()
    } else {
        crate::daemon::short_hash(value.as_bytes())
    }
}

fn read_guard_state(path: &Path) -> Result<Option<GuardState>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(serde_json::from_str(&text)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn write_guard_state(path: &Path, state: &GuardState) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec(state)?)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::remove_file(path)?;
            std::fs::rename(&tmp, path)?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e.into())
        }
    }
}

struct StateLock {
    path: PathBuf,
    _file: File,
}

impl StateLock {
    fn acquire(state_path: &Path) -> Result<Self> {
        let lock_path = state_path.with_extension("lock");
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)?;
        Ok(Self {
            path: lock_path,
            _file: file,
        })
    }
}

impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn cleanup_turn_guard_state_best_effort(session: &str) {
    let Ok(paths) = crate::daemon::DaemonPaths::current() else {
        return;
    };
    let root = paths.run_dir.join("copilot").join("turn-guard");
    let file_name = format!("{}.json", path_token(session));
    if let Ok(store_dirs) = std::fs::read_dir(&root) {
        for entry in store_dirs.flatten() {
            let path = entry.path().join(&file_name);
            let _ = std::fs::remove_file(path);
            let lock_path = entry
                .path()
                .join(format!("{}.lock", path_token(session)));
            let _ = std::fs::remove_file(lock_path);
        }
    }
}

#[derive(Debug, Serialize)]
struct HookLogEvent<'a> {
    ts_ms: i64,
    hook: &'a str,
    reason_code: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nudges: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cap: Option<u32>,
}

impl<'a> HookLogEvent<'a> {
    fn session_end(
        reason_code: &'a str,
        session_id: Option<&'a str>,
        detail: Option<&'a str>,
    ) -> Self {
        Self {
            ts_ms: now_ms(),
            hook: "sessionEnd",
            reason_code,
            session_id,
            detail,
            nudges: None,
            cap: None,
        }
    }

    fn turn_guard(
        reason_code: &'a str,
        session_id: Option<&'a str>,
        detail: Option<&'a str>,
        nudges: u32,
        cap: u32,
    ) -> Self {
        Self {
            ts_ms: now_ms(),
            hook: "agentStop",
            reason_code,
            session_id,
            detail,
            nudges: Some(nudges),
            cap: Some(cap),
        }
    }
}

fn write_hook_log_best_effort(event: &HookLogEvent<'_>) {
    let Ok(path) = hook_log_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        if let Ok(line) = serde_json::to_string(event) {
            let _ = writeln!(file, "{line}");
        }
    }
}

fn print_json(value: &serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon_ipc::{ProtocolVersion, StationHealth};
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn member(address: &str, live_waiters_count: usize, pending: i64) -> MemberStatus {
        MemberStatus {
            store_key: "sqlite:/tmp/telex.db".to_string(),
            backend: "sqlite".to_string(),
            session_id: "s1".to_string(),
            address: address.to_string(),
            occupant: "tester".to_string(),
            host: "host".to_string(),
            waiters: live_waiters_count,
            live_waiters_count,
            pending_unconsumed_count: pending,
            station_health: if live_waiters_count > 0 {
                StationHealth::Armed
            } else {
                StationHealth::Unattended
            },
            health_detail: None,
            last_waiter_exit_at_ms: None,
            last_waiter_outcome: None,
            last_delivered_message_id: None,
            live_waiters: Vec::new(),
            watch_pids: Vec::new(),
            description: None,
            scope: None,
            tags: None,
            lease_epoch: 1,
            owner_instance_id: "owner".to_string(),
            idle: false,
        }
    }

    fn restore_env(key: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn parses_copilot_session_payload_shapes() {
        assert_eq!(
            parse_session_id(r#"{"sessionId":"abc-123"}"#).as_deref(),
            Some("abc-123")
        );
        assert_eq!(
            parse_session_id(r#"{"data":{"session_id":"nested"}}"#).as_deref(),
            Some("nested")
        );
        assert_eq!(parse_session_id(r#"{"other":"x"}"#), None);
    }

    #[test]
    fn guard_blocks_unarmed_attended_station_with_pending_count() {
        let settings = GuardSettings {
            enabled: true,
            max_nudges: 3,
        };
        let eval = evaluate_guard("s1", &[member("addr:a", 0, 2)], settings, None);
        assert_eq!(eval.reason_code, "unarmed_attended_station");
        assert_eq!(eval.nudges, 1);
        match eval.decision {
            HookDecision::Block { reason } => {
                assert!(reason.contains("addr:a (pending 2)"));
                assert!(reason.contains("Nudge 1/3"));
            }
            other => panic!("expected block, got {other:?}"),
        }
    }

    #[test]
    fn guard_allows_after_cap_exhaustion() {
        let settings = GuardSettings {
            enabled: true,
            max_nudges: 2,
        };
        let prior = Some(GuardState {
            nudges: 2,
            last_decision: "unarmed_attended_station".to_string(),
            updated_at_ms: 1,
        });
        let eval = evaluate_guard("s1", &[member("addr:a", 0, 0)], settings, prior);
        assert_eq!(eval.reason_code, "cap_exhausted");
        assert!(matches!(eval.decision, HookDecision::Allow));
        assert_eq!(eval.next_state.unwrap().nudges, 2);
    }

    #[test]
    fn guard_resets_when_live_waiter_is_observed() {
        let settings = GuardSettings {
            enabled: true,
            max_nudges: 3,
        };
        let prior = Some(GuardState {
            nudges: 3,
            last_decision: "cap_exhausted".to_string(),
            updated_at_ms: 1,
        });
        let eval = evaluate_guard(
            "s1",
            &[member("addr:armed", 1, 0), member("addr:unarmed", 0, 0)],
            settings,
            prior,
        );
        assert_eq!(eval.reason_code, "unarmed_attended_station");
        assert_eq!(eval.next_state.unwrap().nudges, 1);
    }

    #[test]
    fn turn_guard_opt_out_wins_over_invalid_cap() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prior_guard = std::env::var_os("TELEX_TURN_GUARD");
        let prior_cap = std::env::var_os("TELEX_TURN_GUARD_MAX_NUDGES");
        std::env::set_var("TELEX_TURN_GUARD", "off");
        std::env::set_var("TELEX_TURN_GUARD_MAX_NUDGES", "not-a-number");
        let settings = GuardSettings::from_env().expect("opt-out should ignore invalid cap");
        restore_env("TELEX_TURN_GUARD", prior_guard);
        restore_env("TELEX_TURN_GUARD_MAX_NUDGES", prior_cap);
        assert!(!settings.enabled);
        assert_eq!(settings.max_nudges, DEFAULT_TURN_GUARD_MAX_NUDGES);
    }

    #[test]
    fn guard_allows_and_clears_state_when_no_stations() {
        let settings = GuardSettings {
            enabled: true,
            max_nudges: 3,
        };
        let eval = evaluate_guard("s1", &[], settings, None);
        assert_eq!(eval.reason_code, "no_attended_stations");
        assert!(matches!(eval.decision, HookDecision::Allow));
        assert!(eval.next_state.is_none());
    }

    #[test]
    fn active_members_filter_ignores_idle_and_other_sessions() {
        let mut idle = member("idle", 0, 0);
        idle.idle = true;
        let mut other = member("other", 0, 0);
        other.session_id = "s2".to_string();
        let active = member("active", 0, 0);
        let status = DaemonStatus {
            protocol_version: ProtocolVersion { major: 1, minor: 2 },
            daemon_version: "test".to_string(),
            instance_id: "inst".to_string(),
            singleton_key: "singleton".to_string(),
            stores: Vec::new(),
            backoff: Vec::new(),
            recent_errors: Vec::new(),
            epoch_by_address: Vec::new(),
            members: vec![idle, other, active],
            live_waiters: Vec::new(),
            retention: Vec::new(),
            idle_stations: Default::default(),
        };
        let got = active_session_members(&status, "s1");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].address, "active");
    }
}
