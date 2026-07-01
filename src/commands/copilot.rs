//! Hidden Copilot CLI plugin adapter commands.
//!
//! This module is the harness boundary: it reads Copilot hook payloads and `COPILOT_*`
//! environment variables, then maps them to generic telex session/watch-pid inputs. Core daemon
//! protocol and identity helpers intentionally remain unaware of Copilot-specific names.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::cli::{
    AttachArgs, CopilotAttachArgs, CopilotCmd, CopilotDetachArgs, CopilotPushArgs,
    CopilotSessionEndArgs, CopilotTurnGuardArgs, Ctx, DetachArgs,
};
use crate::daemon_ipc::{
    DaemonStatus, MemberStatus, Request, Response, WaiterOutcome, WatchPidSpec,
};
use crate::model::now_ms;

const DEFAULT_TURN_GUARD_MAX_NUDGES: u32 = 3;
const TURN_GUARD_DISABLED: &str = "turn_guard_disabled";
const HOOK_LOG_FILE: &str = "hook-events.ndjson";
const HOOK_LOG_ROTATE_BYTES: u64 = 1_048_576;
const LOCK_STALE_AFTER: Duration = Duration::from_secs(5 * 60);

pub async fn run(ctx: &Ctx, cmd: CopilotCmd) -> Result<i32> {
    match cmd {
        CopilotCmd::Attach(args) => attach(ctx, args).await,
        CopilotCmd::SessionEnd(args) => session_end(ctx, args).await,
        CopilotCmd::TurnGuard(args) => turn_guard(ctx, args).await,
        CopilotCmd::Skill => skill(),
        CopilotCmd::Push(args) => push(ctx, args).await,
        CopilotCmd::Detach(args) => detach(ctx, args).await,
    }
}

/// The bridge extension bytes, embedded so they version with the daemon protocol.
const BRIDGE_EXTENSION_MJS: &str = include_str!("../../copilot-bridge/extension.mjs");
const BRIDGE_EXTENSION_NAME: &str = "telex-bridge";

fn copilot_home_dir() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".copilot"))
        .ok_or_else(|| anyhow::anyhow!("no home directory"))
}

fn bridge_extension_dir(session_id: &str) -> Result<PathBuf> {
    Ok(copilot_home_dir()?
        .join("session-state")
        .join(session_id)
        .join("extensions")
        .join(BRIDGE_EXTENSION_NAME))
}

fn bridge_bindings_path(session_id: &str) -> Result<PathBuf> {
    Ok(copilot_home_dir()?
        .join("telex-bridge")
        .join(format!("{session_id}.bindings.json")))
}

/// Write the embedded bridge extension into the session's extension discovery dir. The agent
/// still runs `extensions_reload` to load it (telex cannot trigger a reload).
fn write_bridge_extension(session_id: &str) -> Result<()> {
    let dir = bridge_extension_dir(session_id)?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("extension.mjs"), BRIDGE_EXTENSION_MJS)?;
    Ok(())
}

fn read_bridge_bindings(session_id: &str) -> Vec<String> {
    bridge_bindings_path(session_id)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
}

/// Record `address` as a bridge binding for the session (ref-count of addresses sharing the
/// one per-session bridge), so teardown only removes the bridge when the last one detaches.
fn add_bridge_binding(session_id: &str, address: &str) -> Result<()> {
    let mut addrs = read_bridge_bindings(session_id);
    if !addrs.iter().any(|a| a == address) {
        addrs.push(address.to_string());
    }
    let path = bridge_bindings_path(session_id)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string(&addrs)?)?;
    Ok(())
}

/// Drop `address` from the session's bridge bindings; return true if none remain (so the
/// bridge extension itself should be removed).
fn remove_bridge_binding(session_id: &str, address: &str) -> Result<bool> {
    let mut addrs = read_bridge_bindings(session_id);
    addrs.retain(|a| a != address);
    if addrs.is_empty() {
        let _ = std::fs::remove_file(bridge_bindings_path(session_id)?);
        Ok(true)
    } else {
        std::fs::write(bridge_bindings_path(session_id)?, serde_json::to_string(&addrs)?)?;
        Ok(false)
    }
}

/// Remove the session's bridge extension, registry, and bindings (best effort). Called on
/// last-binding detach and on session end so a bridge never reloads on a later resume.
fn remove_bridge_extension(session_id: &str) {
    if let Ok(dir) = bridge_extension_dir(session_id) {
        let _ = std::fs::remove_dir_all(dir);
    }
    if let Ok(registry) = bridge_registry_path(session_id) {
        let _ = std::fs::remove_file(registry);
    }
    if let Ok(bindings) = bridge_bindings_path(session_id) {
        let _ = std::fs::remove_file(bindings);
    }
}

fn bridge_handler_argv(session_id: &str) -> Result<Vec<String>> {
    let exe = std::env::current_exe()?.to_string_lossy().to_string();
    Ok(vec![
        exe,
        "copilot".to_string(),
        "push".to_string(),
        "--session".to_string(),
        session_id.to_string(),
    ])
}

/// On `--copilot-bridge` bind: materialize the bridge, record the binding, and return the
/// on-deliver handler argv the daemon should exec for this address. Returns None (no push
/// registration) if the bridge could not be provisioned.
fn provision_bridge(ctx: &Ctx, session_id: &str) -> Option<Vec<String>> {
    let address = match ctx.cfg.require_address(&ctx.address) {
        Ok(address) => address,
        Err(e) => {
            eprintln!("telex copilot attach: --copilot-bridge needs an address: {e}");
            return None;
        }
    };
    if let Err(e) = write_bridge_extension(session_id) {
        eprintln!("telex copilot attach: failed to write bridge extension: {e}");
        return None;
    }
    if let Err(e) = add_bridge_binding(session_id, &address) {
        eprintln!("telex copilot attach: failed to record bridge binding: {e}");
    }
    match bridge_handler_argv(session_id) {
        Ok(argv) => Some(argv),
        Err(e) => {
            eprintln!("telex copilot attach: {e}");
            None
        }
    }
}

/// `telex copilot detach`: generic address detach plus bridge teardown when this was the
/// session's last bridge binding.
async fn detach(ctx: &Ctx, args: CopilotDetachArgs) -> Result<i32> {
    let session = match resolve_copilot_session(args.session.as_deref(), None) {
        Some(session) => session,
        None => {
            eprintln!(
                "telex: no Copilot session id available; set COPILOT_AGENT_SESSION_ID or pass --session"
            );
            return Ok(1);
        }
    };
    let address = ctx.cfg.require_address(&ctx.address).ok();
    let code = crate::commands::detach::run(
        ctx,
        DetachArgs {
            session: Some(session.clone()),
        },
    )
    .await?;
    if let Some(address) = address {
        if let Ok(true) = remove_bridge_binding(&session, &address) {
            remove_bridge_extension(&session);
        }
    }
    Ok(code)
}

/// Harness-neutral message descriptor the daemon's on-deliver exec feeds on stdin.
#[derive(Deserialize)]
struct OnDeliverDescriptor {
    message_id: i64,
    address: String,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    attention: String,
    #[serde(default)]
    requires_disposition: bool,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    body: String,
}

/// The bridge registry entry the in-session extension writes for its session.
#[derive(Deserialize)]
struct BridgeRegistry {
    endpoint: BridgeEndpoint,
}

#[derive(Deserialize)]
struct BridgeEndpoint {
    #[allow(dead_code)]
    kind: String,
    path: String,
}

#[derive(Serialize)]
struct BridgePushRequest {
    prompt: String,
    #[serde(rename = "displayPrompt")]
    display_prompt: String,
    mode: &'static str,
}

#[derive(Deserialize)]
struct BridgePushResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

/// Locked two-mode mapping (#53): `interrupt` steers the running turn (`immediate`);
/// every other attention level waits for the next turn boundary (`enqueue`).
fn attention_to_send_mode(attention: &str) -> &'static str {
    if attention == "interrupt" {
        "immediate"
    } else {
        "enqueue"
    }
}

fn bridge_registry_path(session_id: &str) -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    Ok(home
        .join(".copilot")
        .join("telex-bridge")
        .join(format!("{session_id}.json")))
}

/// Compose the prompt the agent sees for a pushed telex message: enough context to act
/// and to record disposition by id (the durable ack stays agent-driven).
fn build_push_prompt(d: &OnDeliverDescriptor) -> String {
    let from = d.from.as_deref().unwrap_or("unknown");
    let mut p = String::new();
    p.push_str("[telex message]\n");
    p.push_str(&format!("from: {from}\n"));
    p.push_str(&format!("to (your address): {}\n", d.address));
    p.push_str(&format!("id: {}\n", d.message_id));
    p.push_str(&format!("attention: {}\n", d.attention));
    if !d.kind.is_empty() {
        p.push_str(&format!("kind: {}\n", d.kind));
    }
    if let Some(subject) = d.subject.as_deref().filter(|s| !s.is_empty()) {
        p.push_str(&format!("subject: {subject}\n"));
    }
    p.push_str(&format!("requires_disposition: {}\n\n", d.requires_disposition));
    p.push_str(&d.body);
    p.push_str(&format!(
        "\n\nThis message was pushed by telex. Record consumption with `telex ack --address {} --id {}`",
        d.address, d.message_id
    ));
    if d.requires_disposition {
        p.push_str(&format!(
            ", then a terminal disposition (`telex handle|reject|close --address {} --id {}`)",
            d.address, d.message_id
        ));
    }
    p.push_str(". Dedupe by id if you have already seen it.");
    p
}

/// `telex copilot push --session <id>`: the daemon's registered on-deliver handler.
/// Reads a message descriptor from stdin, resolves the session's bridge endpoint from the
/// registry, and hands the message to the in-session bridge over the local pipe/socket.
/// Exit 0 only when the bridge accepted it (session.send succeeded); any non-zero exit
/// leaves the message durably unacked so the daemon retries. Never acks telex.
async fn push(_ctx: &Ctx, args: CopilotPushArgs) -> Result<i32> {
    let session = match resolve_copilot_session(args.session.as_deref(), None) {
        Some(session) => session,
        None => {
            eprintln!("telex copilot push: no Copilot session id; set COPILOT_AGENT_SESSION_ID or --session");
            return Ok(2);
        }
    };

    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() || input.trim().is_empty() {
        eprintln!("telex copilot push: empty message descriptor on stdin");
        return Ok(2);
    }
    let descriptor: OnDeliverDescriptor = match serde_json::from_str(input.trim()) {
        Ok(descriptor) => descriptor,
        Err(e) => {
            eprintln!("telex copilot push: malformed descriptor: {e}");
            return Ok(2);
        }
    };

    let registry_path = bridge_registry_path(&session)?;
    let registry: BridgeRegistry = match std::fs::read_to_string(&registry_path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
    {
        Some(registry) => registry,
        None => {
            eprintln!(
                "telex copilot push: no live bridge for session {session} at {}",
                registry_path.display()
            );
            return Ok(2);
        }
    };

    let request = BridgePushRequest {
        prompt: build_push_prompt(&descriptor),
        display_prompt: format!(
            "[telex] from {} ({})",
            descriptor.from.as_deref().unwrap_or("unknown"),
            descriptor.attention
        ),
        mode: attention_to_send_mode(&descriptor.attention),
    };
    let line = serde_json::to_string(&request)?;

    let response =
        match tokio::time::timeout(Duration::from_secs(20), bridge_roundtrip(&registry.endpoint.path, &line))
            .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(e)) => {
                eprintln!("telex copilot push: bridge transport failed: {e}");
                return Ok(2);
            }
            Err(_) => {
                eprintln!("telex copilot push: bridge did not respond within budget");
                return Ok(2);
            }
        };

    let parsed: BridgePushResponse = match serde_json::from_str(response.trim()) {
        Ok(parsed) => parsed,
        Err(e) => {
            eprintln!("telex copilot push: malformed bridge response: {e}");
            return Ok(1);
        }
    };
    if parsed.ok {
        Ok(0)
    } else {
        eprintln!(
            "telex copilot push: bridge rejected message {}: {}",
            descriptor.message_id,
            parsed.error.as_deref().unwrap_or("unknown error")
        );
        Ok(1)
    }
}

/// Connect to the in-session bridge endpoint, send one JSON request line, read one JSON
/// response line. Windows named pipe path.
#[cfg(windows)]
async fn bridge_roundtrip(path: &str, request_line: &str) -> Result<String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::ClientOptions;
    const ERROR_PIPE_BUSY: i32 = 231;

    let mut client = loop {
        match ClientOptions::new().open(path) {
            Ok(client) => break client,
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => return Err(anyhow::anyhow!("opening bridge pipe {path}: {e}")),
        }
    };
    client.write_all(request_line.as_bytes()).await?;
    client.write_all(b"\n").await?;
    client.flush().await?;
    let mut reader = BufReader::new(client);
    let mut response = String::new();
    reader.read_line(&mut response).await?;
    Ok(response)
}

/// POSIX unix domain socket path.
#[cfg(unix)]
async fn bridge_roundtrip(path: &str, request_line: &str) -> Result<String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let mut client = UnixStream::connect(path)
        .await
        .map_err(|e| anyhow::anyhow!("connecting bridge socket {path}: {e}"))?;
    client.write_all(request_line.as_bytes()).await?;
    client.write_all(b"\n").await?;
    client.flush().await?;
    let mut reader = BufReader::new(client);
    let mut response = String::new();
    reader.read_line(&mut response).await?;
    Ok(response)
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
    let on_deliver = if args.copilot_bridge {
        provision_bridge(ctx, &session)
    } else {
        None
    };
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
        on_deliver,
    };
    crate::commands::attach::run(ctx, attach_args).await
}

async fn session_end(ctx: &Ctx, args: CopilotSessionEndArgs) -> Result<i32> {
    let payload = read_stdin_payload();
    let session = match resolve_copilot_session(args.session.as_deref(), payload.as_deref()) {
        Some(session) => session,
        None => {
            let reason_code = if payload.is_some() {
                "payload_unknown_shape"
            } else {
                "missing_session"
            };
            let event = HookLogEvent::session_end(reason_code, None, None);
            write_hook_log_best_effort(&event);
            print_json(&serde_json::json!({"session_end": false, "outcome": reason_code}));
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

    let (mut client, cap) = match connect_existing_with_cap(&store_key).await {
        Ok(connection) => connection,
        Err(e) => {
            let event = HookLogEvent::session_end("daemon_unavailable", Some(&session), Some(&e));
            write_hook_log_best_effort(&event);
            print_json(
                &serde_json::json!({"session_end": false, "session_id": session, "store_key": store_key, "outcome": "daemon_unavailable"}),
            );
            return Ok(0);
        }
    };

    let mut ended = Vec::new();
    let mut failed = Vec::new();
    let response = client
        .request(&Request::SessionEnd {
            store_key: store_key.clone(),
            session_id: session.clone(),
            proof: Some(cap.admin_cap),
        })
        .await;
    match response {
        Ok(Response::Ack { .. }) => ended.push(store_key.clone()),
        Ok(Response::Error { code, message, .. }) => {
            failed.push(format!("{store_key}: {code}: {message}"));
        }
        Ok(other) => failed.push(format!("{store_key}: unexpected {other:?}")),
        Err(e) => failed.push(format!("{store_key}: {e}")),
    }

    cleanup_turn_guard_state_best_effort(&session);
    remove_bridge_extension(&session);
    let outcome = if failed.is_empty() {
        "session_end"
    } else {
        "partial_session_end"
    };
    let failure_detail = (!failed.is_empty()).then(|| failed.join("; "));
    let event = HookLogEvent::session_end(outcome, Some(&session), failure_detail.as_deref());
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
            let reason_code = if payload.is_some() {
                "payload_unknown_shape"
            } else {
                "missing_session"
            };
            return allow_with_log(None, reason_code, "No Copilot session id was available.");
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

    let active_members = active_session_members(&status, &store_key, &session);
    let state_path = turn_guard_state_path(&session)?;
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

fn skill() -> Result<i32> {
    print!("{}", crate::commands::skill::raw_skill());
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

fn active_session_members(
    status: &DaemonStatus,
    store_key: &str,
    session: &str,
) -> Vec<MemberStatus> {
    status
        .members
        .iter()
        .filter(|member| {
            member.store_key == store_key && member.session_id == session && !member.idle
        })
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
        let enabled = !matches!(
            env_nonempty("TELEX_TURN_GUARD")
                .map(|value| value.to_ascii_lowercase())
                .as_deref(),
            Some("off" | "0" | "false")
        );
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    issue_key: Option<String>,
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
    let delivered_unacked = members
        .iter()
        .filter(|member| {
            member.pending_unconsumed_count > 0
                && member.last_waiter_outcome == Some(WaiterOutcome::Message)
        })
        .collect::<Vec<_>>();
    if unarmed.is_empty() && delivered_unacked.is_empty() {
        return GuardEvaluation {
            decision: HookDecision::Allow,
            reason_code: "covered",
            summary: "All attended stations are covered.".to_string(),
            nudges: 0,
            next_state: None,
        };
    }

    let issue_key = coverage_issue_key(&unarmed, &delivered_unacked);
    let prior_nudges = match prior_state {
        Some(state) if state.issue_key.as_deref() == Some(issue_key.as_str()) => state.nudges,
        _ => 0,
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
                issue_key: Some(issue_key),
            }),
        };
    }

    let nudges = prior_nudges.saturating_add(1);
    let station_list = coverage_summary(&unarmed, &delivered_unacked);
    let guidance = if !unarmed.is_empty() && !delivered_unacked.is_empty() {
        "Ack handled deliveries, then re-arm `telex wait ... --out-dir <dir>` if still attending, or run `telex detach --address <station>` if done."
    } else if !unarmed.is_empty() {
        "Re-arm `telex wait ... --out-dir <dir>` if still attending, or run `telex detach --address <station>` if done."
    } else {
        "Ack handled deliveries with `telex ack --address <station> --session <session-id> --id <message-id>` before ending the turn; unacked messages redeliver."
    };
    let reason = format!(
        "Telex turn guard: session {session} has uncovered station work: {station_list}. {guidance} Nudge {nudges}/{}.",
        settings.max_nudges
    );
    GuardEvaluation {
        decision: HookDecision::Block { reason },
        reason_code: "coverage_gap",
        summary: station_list,
        nudges,
        next_state: Some(GuardState {
            nudges,
            last_decision: "coverage_gap".to_string(),
            updated_at_ms: now_ms(),
            issue_key: Some(issue_key),
        }),
    }
}

fn coverage_summary(unarmed: &[&MemberStatus], delivered_unacked: &[&MemberStatus]) -> String {
    let mut parts = Vec::new();
    parts.extend(unarmed.iter().map(|member| {
        format!(
            "{} has no live waiter (pending {})",
            member.address, member.pending_unconsumed_count
        )
    }));
    parts.extend(delivered_unacked.iter().map(|member| {
        format!(
            "{} has {} delivered/unacked message(s)",
            member.address, member.pending_unconsumed_count
        )
    }));
    parts.join(", ")
}

fn coverage_issue_key(unarmed: &[&MemberStatus], delivered_unacked: &[&MemberStatus]) -> String {
    let mut parts = Vec::new();
    parts.extend(
        unarmed
            .iter()
            .map(|member| format!("unarmed\0{}\0{}", member.store_key, member.address)),
    );
    parts.extend(
        delivered_unacked
            .iter()
            .map(|member| format!("unacked\0{}\0{}", member.store_key, member.address)),
    );
    parts.sort();
    parts.join("\n")
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

fn turn_guard_state_path(session: &str) -> Result<PathBuf> {
    let paths = crate::daemon::DaemonPaths::current()?;
    Ok(paths
        .run_dir
        .join("copilot")
        .join("turn-guard")
        .join(format!("{}.json", path_token(session))))
}

fn hook_log_path() -> Result<PathBuf> {
    let paths = crate::daemon::DaemonPaths::current()?;
    Ok(paths.run_dir.join("copilot").join(HOOK_LOG_FILE))
}

fn path_token(value: &str) -> String {
    if value.len() <= 80
        && value
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
        let file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(file) => file,
            Err(e)
                if e.kind() == std::io::ErrorKind::AlreadyExists
                    && Self::stale_lock(&lock_path) =>
            {
                let _ = std::fs::remove_file(&lock_path);
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&lock_path)?
            }
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            path: lock_path,
            _file: file,
        })
    }

    fn stale_lock(path: &Path) -> bool {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|elapsed| elapsed > LOCK_STALE_AFTER)
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
    let file_stem = path_token(session);
    let _ = std::fs::remove_file(root.join(format!("{file_stem}.json")));
    let _ = std::fs::remove_file(root.join(format!("{file_stem}.lock")));
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
    rotate_hook_log_best_effort(&path);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        if let Ok(line) = serde_json::to_string(event) {
            let _ = writeln!(file, "{line}");
        }
    }
}

fn rotate_hook_log_best_effort(path: &Path) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    if metadata.len() < HOOK_LOG_ROTATE_BYTES {
        return;
    }
    let rotated = path.with_extension("ndjson.1");
    let _ = std::fs::remove_file(&rotated);
    let _ = std::fs::rename(path, rotated);
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
            last_waiter_exit_code: None,
            last_waiter_detail: None,
            last_waiter_pid: None,
            last_delivered_message_id: None,
            unattended_since_ms: None,
            unattended_for_ms: None,
            deaf_since_ms: None,
            deaf_for_ms: None,
            deaf_warn: false,
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

    fn member_in_store(
        store_key: &str,
        session: &str,
        address: &str,
        live_waiters_count: usize,
        pending: i64,
    ) -> MemberStatus {
        let mut member = member(address, live_waiters_count, pending);
        member.store_key = store_key.to_string();
        member.session_id = session.to_string();
        member
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
        assert_eq!(eval.reason_code, "coverage_gap");
        assert_eq!(eval.nudges, 1);
        match eval.decision {
            HookDecision::Block { reason } => {
                assert!(reason.contains("addr:a has no live waiter (pending 2)"));
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
            last_decision: "coverage_gap".to_string(),
            updated_at_ms: 1,
            issue_key: Some(coverage_issue_key(&[&member("addr:a", 0, 0)], &[])),
        });
        let eval = evaluate_guard("s1", &[member("addr:a", 0, 0)], settings, prior);
        assert_eq!(eval.reason_code, "cap_exhausted");
        assert!(matches!(eval.decision, HookDecision::Allow));
        assert_eq!(eval.next_state.unwrap().nudges, 2);
    }

    #[test]
    fn guard_counts_persistent_unarmed_set_even_with_other_live_waiter() {
        let settings = GuardSettings {
            enabled: true,
            max_nudges: 3,
        };
        let armed = member("addr:armed", 1, 0);
        let unarmed = member("addr:unarmed", 0, 0);
        let prior = Some(GuardState {
            nudges: 2,
            last_decision: "coverage_gap".to_string(),
            updated_at_ms: 1,
            issue_key: Some(coverage_issue_key(&[&unarmed], &[])),
        });
        let eval = evaluate_guard("s1", &[armed, unarmed], settings, prior);
        assert_eq!(eval.reason_code, "coverage_gap");
        assert_eq!(eval.next_state.unwrap().nudges, 3);
    }

    #[test]
    fn guard_resets_when_unarmed_station_set_changes() {
        let settings = GuardSettings {
            enabled: true,
            max_nudges: 3,
        };
        let previous = member("addr:old", 0, 0);
        let current = member("addr:new", 0, 0);
        let prior = Some(GuardState {
            nudges: 3,
            last_decision: "cap_exhausted".to_string(),
            updated_at_ms: 1,
            issue_key: Some(coverage_issue_key(&[&previous], &[])),
        });
        let eval = evaluate_guard("s1", &[current], settings, prior);
        assert_eq!(eval.reason_code, "coverage_gap");
        assert_eq!(eval.next_state.unwrap().nudges, 1);
    }

    #[test]
    fn guard_nudges_for_delivered_unacked_message() {
        let settings = GuardSettings {
            enabled: true,
            max_nudges: 3,
        };
        let mut delivered = member("addr:delivered", 1, 1);
        delivered.last_waiter_outcome = Some(WaiterOutcome::Message);
        let eval = evaluate_guard("s1", &[delivered], settings, None);
        assert_eq!(eval.reason_code, "coverage_gap");
        match eval.decision {
            HookDecision::Block { reason } => {
                assert!(reason.contains("delivered/unacked"));
                assert!(reason.contains("Ack handled deliveries"));
            }
            other => panic!("expected block, got {other:?}"),
        }
    }

    #[test]
    fn guard_does_not_nudge_for_inflight_pending_without_delivery_exit() {
        let settings = GuardSettings {
            enabled: true,
            max_nudges: 3,
        };
        let pending_with_waiter = member("addr:pending", 1, 1);
        let eval = evaluate_guard("s1", &[pending_with_waiter], settings, None);
        assert_eq!(eval.reason_code, "covered");
        assert!(matches!(eval.decision, HookDecision::Allow));
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
    fn path_token_hashes_overlong_safe_session_ids() {
        let long = "a".repeat(300);
        let token = path_token(&long);
        assert_ne!(token, long);
        assert!(token.len() <= 80);
    }

    #[test]
    fn active_members_filter_ignores_idle_other_sessions_and_other_stores() {
        let mut idle = member("idle", 0, 0);
        idle.idle = true;
        let mut other = member("other", 0, 0);
        other.session_id = "s2".to_string();
        let other_store = member_in_store("sqlite:/other.db", "s1", "other-store", 0, 0);
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
            members: vec![idle, other, other_store, active],
            live_waiters: Vec::new(),
            retention: Vec::new(),
            idle_stations: Default::default(),
            deaf_stations: Default::default(),
        };
        let got = active_session_members(&status, "sqlite:/tmp/telex.db", "s1");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].address, "active");
    }
}
