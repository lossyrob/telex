//! Conservative observation of the private, same-user Copilot CLI event format.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

pub const TAIL_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionState {
    Busy,
    Idle,
    WaitingOnOperator,
    NotLoaded,
    Unknown,
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Busy => "busy",
            Self::Idle => "idle",
            Self::WaitingOnOperator => "waiting-on-operator",
            Self::NotLoaded => "not-loaded",
            Self::Unknown => "unknown",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActivityMarker {
    pub id: String,
    pub timestamp: String,
}

#[derive(Debug, Serialize)]
pub struct Classification {
    pub session_id: String,
    pub state: SessionState,
    pub reason: &'static str,
    /// Last non-accounting event, not the read time or file modification time.
    pub activity: Option<ActivityMarker>,
}

impl Classification {
    fn unknown(session_id: &str, reason: &'static str) -> Self {
        Self {
            session_id: session_id.to_owned(),
            state: SessionState::Unknown,
            reason,
            activity: None,
        }
    }
}

pub fn parse_session_id(value: &str) -> std::result::Result<String, String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("session id must be 1-128 ASCII letters, digits, '-' or '_'".into());
    }
    Ok(value.to_owned())
}

/// Read only the current user's local session directory. No daemon, lock or write.
pub fn classify_local(session_id: &str) -> Result<Classification> {
    parse_session_id(session_id).map_err(|e| anyhow!(e))?;
    let home = match super::copilot_home_dir() {
        Ok(home) => home,
        Err(_) => return Ok(Classification::unknown(session_id, "home-unavailable")),
    };
    Ok(classify_path(
        session_id,
        &home
            .join("session-state")
            .join(session_id)
            .join("events.jsonl"),
    ))
}

fn classify_path(session_id: &str, path: &Path) -> Classification {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Classification::unknown(session_id, "log-missing");
        }
        Err(_) => return Classification::unknown(session_id, "log-unreadable"),
    };
    let metadata = match file.metadata() {
        Ok(metadata) if metadata.is_file() => metadata,
        _ => return Classification::unknown(session_id, "log-unreadable"),
    };
    match read_tail(file, metadata.len()) {
        Ok(tail) => classify_tail(session_id, &tail),
        Err(_) => Classification::unknown(session_id, "log-read-failed"),
    }
}

fn read_tail(mut reader: impl Read + Seek, len: u64) -> io::Result<Vec<u8>> {
    let start = len.saturating_sub(TAIL_BYTES as u64);
    reader.seek(SeekFrom::Start(start))?;
    let mut tail = vec![0; (len - start) as usize];
    // A fixed snapshot length excludes concurrent appends and detects truncation.
    reader.read_exact(&mut tail)?;
    if start != 0 {
        let boundary = tail
            .iter()
            .position(|b| *b == b'\n')
            .map_or(tail.len(), |i| i + 1);
        tail.drain(..boundary);
    }
    Ok(tail)
}

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: String,
    id: String,
    timestamp: String,
    data: Value,
    #[serde(rename = "agentId")]
    agent_id: Option<String>,
}

fn field<'a>(data: &'a Value, name: &str) -> Option<&'a str> {
    data.get(name)?.as_str().filter(|s| !s.is_empty())
}

fn classify_tail(session_id: &str, tail: &[u8]) -> Classification {
    let mut result = Classification::unknown(session_id, "no-state-evidence");
    if tail.is_empty() {
        return result;
    }
    if !tail.ends_with(b"\n") {
        return Classification::unknown(session_id, "incomplete-tail");
    }
    let text = match std::str::from_utf8(tail) {
        Ok(text) => text,
        Err(_) => return Classification::unknown(session_id, "invalid-utf8"),
    };
    let mut tools = HashMap::new();
    let mut stop_hooks = HashSet::new();
    let mut invalidated_stops = HashSet::new();
    let mut stop_blocked = false;
    for line in text.lines() {
        let event: Event = match serde_json::from_str(line) {
            Ok(event) => event,
            Err(_) => return Classification::unknown(session_id, "malformed-event"),
        };
        if event.id.is_empty() || event.timestamp.is_empty() || !event.data.is_object() {
            return Classification::unknown(session_id, "malformed-event");
        }
        if matches!(
            event.kind.as_str(),
            "session.usage_checkpoint" | "session.usage_record"
        ) {
            continue;
        }
        result.activity = Some(ActivityMarker {
            id: event.id,
            timestamp: event.timestamp,
        });
        let (state, reason) = match event.kind.as_str() {
            "user.message" | "assistant.turn_start" => {
                if event.agent_id.is_none() {
                    stop_blocked = false;
                    invalidated_stops.extend(stop_hooks.drain());
                }
                (SessionState::Busy, "activity-started")
            }
            "tool.execution_start" => {
                let (Some(id), Some(name)) = (
                    field(&event.data, "toolCallId"),
                    field(&event.data, "toolName"),
                ) else {
                    return Classification::unknown(session_id, "malformed-tool-start");
                };
                if tools.insert(id.to_owned(), name == "ask_user").is_some() {
                    return Classification::unknown(session_id, "duplicate-tool-start");
                }
                (SessionState::Busy, "tool-running")
            }
            "tool.execution_complete" => {
                let Some(id) = field(&event.data, "toolCallId") else {
                    return Classification::unknown(session_id, "malformed-tool-completion");
                };
                if !event.data.get("success").is_some_and(Value::is_boolean) {
                    return Classification::unknown(session_id, "malformed-tool-completion");
                }
                if tools.remove(id).is_some() {
                    (SessionState::Busy, "tool-completed")
                } else {
                    (SessionState::Unknown, "unpaired-tool-completion")
                }
            }
            "session.shutdown" if event.agent_id.is_none() => {
                tools.clear();
                invalidated_stops.extend(stop_hooks.drain());
                (SessionState::NotLoaded, "session-shutdown")
            }
            "session.resume" if event.agent_id.is_none() => {
                tools.clear();
                invalidated_stops.extend(stop_hooks.drain());
                stop_blocked = false;
                (SessionState::Unknown, "session-resumed")
            }
            "hook.start"
                if field(&event.data, "hookType") == Some("agentStop")
                    && event.agent_id.is_none() =>
            {
                let Some(id) = field(&event.data, "hookInvocationId") else {
                    return Classification::unknown(session_id, "malformed-stop-hook");
                };
                if invalidated_stops.contains(id) || !stop_hooks.insert(id.to_owned()) {
                    return Classification::unknown(session_id, "duplicate-stop-hook");
                }
                (SessionState::Unknown, "stop-hook-pending")
            }
            "hook.end"
                if field(&event.data, "hookType") == Some("agentStop")
                    && event.agent_id.is_none() =>
            {
                let Some(id) = field(&event.data, "hookInvocationId") else {
                    return Classification::unknown(session_id, "malformed-stop-hook");
                };
                stop_hooks.remove(id);
                if invalidated_stops.contains(id) {
                    (SessionState::Unknown, "agent-stop-invalidated")
                } else if event.data.get("success") == Some(&Value::Bool(true))
                    && event
                        .data
                        .pointer("/output/decision")
                        .and_then(Value::as_str)
                        == Some("allow")
                    && !stop_blocked
                    && stop_hooks.is_empty()
                {
                    (SessionState::Idle, "agent-stop-allowed")
                } else {
                    if event.data.get("success") != Some(&Value::Bool(true))
                        || event
                            .data
                            .pointer("/output/decision")
                            .and_then(Value::as_str)
                            != Some("allow")
                    {
                        stop_blocked = true;
                    }
                    (SessionState::Unknown, "agent-stop-not-allowed")
                }
            }
            "assistant.turn_end" | "assistant.message" | "hook.start" | "hook.end" => {
                if result.state == SessionState::Busy
                    || result.state == SessionState::WaitingOnOperator
                {
                    (SessionState::Busy, "turn-in-progress")
                } else {
                    (SessionState::Unknown, "no-stop-evidence")
                }
            }
            _ => (SessionState::Unknown, "unrecognized-event"),
        };
        (result.state, result.reason) = (state, reason);
        if result.state != SessionState::Unknown && result.state != SessionState::NotLoaded {
            if tools.values().any(|waiting| *waiting) {
                (result.state, result.reason) =
                    (SessionState::WaitingOnOperator, "ask-user-pending");
            } else if !tools.is_empty() {
                (result.state, result.reason) = (SessionState::Busy, "tool-running");
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    fn event(kind: &str, data: Value) -> Value {
        json!({
            "type": kind, "id": format!("id-{kind}"),
            "timestamp": "2026-10-25T20:00:00Z", "data": data
        })
    }

    fn stop(decision: &str, success: bool) -> Value {
        event(
            "hook.end",
            json!({
                "hookType": "agentStop", "hookInvocationId": "stop-1",
                "success": success, "output": {"decision": decision}
            }),
        )
    }

    fn start(id: &str, name: &str) -> Value {
        event(
            "tool.execution_start",
            json!({"toolCallId": id, "toolName": name}),
        )
    }

    fn complete(id: &str) -> Value {
        event(
            "tool.execution_complete",
            json!({"toolCallId": id, "success": true}),
        )
    }

    fn classify(events: &[Value]) -> Classification {
        let mut tail = Vec::new();
        for event in events {
            serde_json::to_writer(&mut tail, event).unwrap();
            tail.push(b'\n');
        }
        classify_tail("test-session", &tail)
    }

    #[test]
    fn session_state_fixtures_cover_each_state_and_malformed_evidence() {
        for (fixture, state, reason) in [
            (
                include_bytes!("../../../tests/fixtures/copilot-session-state/busy.jsonl")
                    .as_slice(),
                SessionState::Busy,
                "activity-started",
            ),
            (
                include_bytes!("../../../tests/fixtures/copilot-session-state/idle.jsonl")
                    .as_slice(),
                SessionState::Idle,
                "agent-stop-allowed",
            ),
            (
                include_bytes!("../../../tests/fixtures/copilot-session-state/waiting.jsonl")
                    .as_slice(),
                SessionState::WaitingOnOperator,
                "ask-user-pending",
            ),
            (
                include_bytes!("../../../tests/fixtures/copilot-session-state/not-loaded.jsonl")
                    .as_slice(),
                SessionState::NotLoaded,
                "session-shutdown",
            ),
            (
                include_bytes!("../../../tests/fixtures/copilot-session-state/unknown.jsonl")
                    .as_slice(),
                SessionState::Unknown,
                "unrecognized-event",
            ),
            (
                include_bytes!("../../../tests/fixtures/copilot-session-state/malformed.jsonl")
                    .as_slice(),
                SessionState::Unknown,
                "malformed-event",
            ),
        ] {
            let result = classify_tail("fixture", fixture);
            assert_eq!((result.state, result.reason), (state, reason));
        }
    }

    #[test]
    fn session_state_idle_requires_an_explicit_successful_root_stop() {
        for events in [
            vec![event("assistant.turn_end", json!({}))],
            vec![complete("outside-tail")],
            vec![stop("block", true)],
            vec![stop("allow", false)],
            vec![stop("allow", true), event("assistant.turn_end", json!({}))],
            vec![stop("allow", true), event("new.private.event", json!({}))],
            vec![stop("block", true), stop("allow", true)],
            vec![event(
                "hook.end",
                json!({"hookType": "agentStop", "success": true}),
            )],
        ] {
            assert_eq!(classify(&events).state, SessionState::Unknown, "{events:?}");
        }
        let mut subagent = stop("allow", true);
        subagent["agentId"] = json!("child");
        assert_eq!(classify(&[subagent]).state, SessionState::Unknown);
        let result = classify(&[
            event("assistant.turn_start", json!({})),
            event("assistant.turn_end", json!({})),
        ]);
        assert_eq!(result.state, SessionState::Busy);
        assert_eq!(
            classify(&[
                stop("block", true),
                event("assistant.turn_start", json!({})),
                stop("allow", true)
            ])
            .state,
            SessionState::Idle
        );
    }

    #[test]
    fn session_state_pairs_parallel_tools_by_id_and_never_infers_idle_from_completion() {
        assert_eq!(classify(&[start("a", "shell")]).state, SessionState::Busy);
        assert_eq!(
            classify(&[start("a", "ask_user")]).state,
            SessionState::WaitingOnOperator
        );
        assert_eq!(
            classify(&[start("a", "ask_user"), start("b", "shell"), complete("b")]).state,
            SessionState::WaitingOnOperator
        );
        assert_eq!(
            classify(&[start("a", "ask_user"), start("b", "shell"), complete("a")]).state,
            SessionState::Busy
        );
        assert_eq!(
            classify(&[start("a", "ask_user"), complete("a")]).state,
            SessionState::Busy
        );
        assert_eq!(
            classify(&[start("a", "shell"), stop("allow", true)]).state,
            SessionState::Busy
        );
        assert_eq!(
            classify(&[start("a", "ask_user"), stop("allow", true)]).state,
            SessionState::WaitingOnOperator
        );
        assert_eq!(
            classify(&[start("a", "shell"), complete("a"), stop("allow", true)]).state,
            SessionState::Idle
        );
        assert_eq!(
            classify(&[start("a", "shell"), start("a", "shell")]).reason,
            "duplicate-tool-start"
        );
        assert_eq!(
            classify(&[event(
                "tool.execution_start",
                json!({"toolName": "ask_user"})
            )])
            .state,
            SessionState::Unknown
        );
        assert_eq!(
            classify(&[event("tool.execution_complete", json!({"toolCallId": "a"}))]).state,
            SessionState::Unknown
        );
    }

    #[test]
    fn session_state_waits_for_all_stop_hooks_and_keeps_a_block_decision() {
        let hook_start = |id| {
            event(
                "hook.start",
                json!({"hookType":"agentStop","hookInvocationId":id}),
            )
        };
        let mut second_end = stop("allow", true);
        second_end["data"]["hookInvocationId"] = json!("stop-2");
        assert_eq!(
            classify(&[hook_start("stop-1")]).state,
            SessionState::Unknown
        );
        assert_eq!(
            classify(&[
                hook_start("stop-1"),
                hook_start("stop-2"),
                stop("allow", true)
            ])
            .state,
            SessionState::Unknown
        );
        assert_eq!(
            classify(&[
                hook_start("stop-1"),
                hook_start("stop-2"),
                stop("allow", true),
                second_end.clone()
            ])
            .state,
            SessionState::Idle
        );
        assert_eq!(
            classify(&[
                hook_start("stop-1"),
                hook_start("stop-2"),
                stop("block", true),
                second_end
            ])
            .state,
            SessionState::Unknown
        );
    }

    #[test]
    fn session_state_shutdown_is_cleared_by_resume_but_resume_is_not_idle() {
        let shutdown = event("session.shutdown", json!({}));
        let resume = event("session.resume", json!({}));
        assert_eq!(classify(&[shutdown.clone()]).state, SessionState::NotLoaded);
        assert_eq!(
            classify(&[shutdown.clone(), resume.clone()]).state,
            SessionState::Unknown
        );
        assert_eq!(
            classify(&[shutdown, resume, event("user.message", json!({}))]).state,
            SessionState::Busy
        );
    }

    #[test]
    fn session_state_late_invalidated_stop_cannot_override_newer_root_activity() {
        let old_start = event(
            "hook.start",
            json!({"hookType": "agentStop", "hookInvocationId": "stop-1"}),
        );
        let mut new_start = old_start.clone();
        new_start["data"]["hookInvocationId"] = json!("stop-2");
        let mut new_end = stop("allow", true);
        new_end["data"]["hookInvocationId"] = json!("stop-2");
        for activity in [
            event("user.message", json!({})),
            event("assistant.turn_start", json!({})),
            event("session.resume", json!({})),
        ] {
            let stale = vec![old_start.clone(), activity, stop("allow", true)];
            let result = classify(&stale);
            assert_eq!(result.state, SessionState::Unknown);
            assert_eq!(result.reason, "agent-stop-invalidated");
            let mut fresh = stale.clone();
            fresh.extend([new_start.clone(), new_end.clone()]);
            assert_eq!(classify(&fresh).state, SessionState::Idle);
            fresh.push(stop("allow", true));
            assert_eq!(
                classify(&fresh).state,
                SessionState::Unknown,
                "a duplicate old completion must remain invalidated"
            );
        }
    }

    #[test]
    fn session_state_front_clipped_stop_remains_decisive_suffix_evidence() {
        for (pending, expected) in [
            (start("ask-1", "ask_user"), SessionState::WaitingOnOperator),
            (
                event(
                    "hook.start",
                    json!({"hookType": "agentStop", "hookInvocationId": "pending-stop"}),
                ),
                SessionState::Unknown,
            ),
        ] {
            let mut accounting = event("session.usage_record", json!({"padding": ""}));
            let encode = |accounting: &Value| {
                let mut bytes = Vec::new();
                for entry in [&pending, accounting, &stop("allow", true)] {
                    serde_json::to_writer(&mut bytes, entry).unwrap();
                    bytes.push(b'\n');
                }
                bytes
            };
            let padding = TAIL_BYTES - encode(&accounting).len();
            for extra in [0, 1] {
                accounting["data"]["padding"] = json!("x".repeat(padding + extra));
                let bytes = encode(&accounting);
                assert_eq!(bytes.len(), TAIL_BYTES + extra);
                let tail = read_tail(Cursor::new(&bytes), bytes.len() as u64).unwrap();
                let result = classify_tail("front-clipped", &tail);
                assert_eq!(
                    result.state,
                    if extra == 0 {
                        expected
                    } else {
                        SessionState::Idle
                    }
                );
                assert_eq!(result.activity.unwrap().id, "id-hook.end");
            }
        }
    }

    #[test]
    fn session_state_bookkeeping_does_not_change_the_activity_marker() {
        let idle = classify(&[stop("allow", true)]);
        let accounting = classify(&[
            stop("allow", true),
            event("session.usage_checkpoint", json!({})),
            event("session.usage_record", json!({})),
        ]);
        assert_eq!(accounting.state, SessionState::Idle);
        assert_eq!(accounting.activity, idle.activity);
        let mut next = stop("allow", true);
        next["id"] = json!("next-stop");
        assert_ne!(classify(&[next]).activity, idle.activity);
        assert_eq!(
            classify(&[event("session.usage_checkpoint", json!({}))]).state,
            SessionState::Unknown
        );
    }

    #[test]
    fn session_state_malformed_or_incomplete_tails_never_reuse_idle() {
        for tail in [
            b"{".as_slice(),
            b"{}\n",
            b"{\"type\":\"hook.end\",\"data\":{}}\n",
            b"\xff\n",
            b"\n",
            b"null\n",
        ] {
            assert_eq!(classify_tail("test", tail).state, SessionState::Unknown);
        }
        let mut tail = serde_json::to_vec(&stop("allow", true)).unwrap();
        assert_eq!(classify_tail("test", &tail).reason, "incomplete-tail");
        tail.extend_from_slice(b"\n{\"type\":");
        assert_eq!(classify_tail("test", &tail).reason, "incomplete-tail");
        tail.extend_from_slice(b"\n");
        assert_eq!(classify_tail("test", &tail).reason, "malformed-event");
    }

    struct CountedReader {
        inner: Cursor<Vec<u8>>,
        read: usize,
    }

    impl Read for CountedReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.read += n;
            Ok(n)
        }
    }

    impl Seek for CountedReader {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.inner.seek(pos)
        }
    }

    #[test]
    fn session_state_reads_at_most_the_fixed_tail_even_for_large_logs() {
        let mut bytes = vec![b'x'; TAIL_BYTES * 4];
        bytes.push(b'\n');
        let expected = include_bytes!("../../../tests/fixtures/copilot-session-state/idle.jsonl");
        bytes.extend_from_slice(expected);
        let len = bytes.len() as u64;
        let mut reader = CountedReader {
            inner: Cursor::new(bytes),
            read: 0,
        };
        let tail = read_tail(&mut reader, len).unwrap();
        assert_eq!(reader.read, TAIL_BYTES);
        assert_eq!(tail, expected);
        assert_eq!(classify_tail("large", &tail).state, SessionState::Idle);

        let mut only_long_line = Cursor::new(vec![b'x'; TAIL_BYTES * 2]);
        assert!(read_tail(&mut only_long_line, (TAIL_BYTES * 2) as u64)
            .unwrap()
            .is_empty());
        assert!(
            read_tail(Cursor::new(b"short"), 100).is_err(),
            "detect a concurrent truncate"
        );
        assert_eq!(
            read_tail(Cursor::new(b"old\nnew\n"), 4).unwrap(),
            b"old\n",
            "exclude appends"
        );
    }

    #[test]
    fn session_state_rejects_paths_and_cross_platform_special_names() {
        for id in [
            "", ".", "..", "../s", r"..\s", "/tmp", r"C:\tmp", "remote:s", "a.b", "a b", "a\nb",
        ] {
            assert!(parse_session_id(id).is_err(), "{id:?}");
        }
        assert!(parse_session_id(&"a".repeat(129)).is_err());
        assert!(parse_session_id("test-session_123").is_ok());
    }
}
