//! CLI-backed port: Station attends its addresses as an ordinary Telex session
//! through the `telex` binary (`attach` / `wait` / `ack` / `reply` / dispositions
//! / `export`). Prototype seam for macOS; see `port.rs` for why.
//!
//! Known gaps versus the Application Client, each surfaced in the UI or design
//! notes rather than papered over:
//!   * no operation ids, so an interrupted send cannot be reconciled exactly;
//!   * `reply` carries no metadata, so replies are not tagged human-authored;
//!   * no `acknowledged` disposition, so "seen by a person" receipts are off;
//!   * recipient consumption is inferred from the recipient's pending count.

use crate::model::*;
use crate::port::{PortError, Reconciled, Sent, TelexPort};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

pub struct CliPort {
    exe: PathBuf,
    session: String,
    backend: Option<String>,
    /// address -> pending_unconsumed_count from the last directory read.
    pending: Mutex<BTreeMap<String, Option<i64>>>,
    attached: Mutex<Vec<String>>,
    cleaned: Mutex<std::collections::BTreeSet<String>>,
}

struct Output {
    code: i32,
    stdout: String,
    stderr: String,
}

fn text(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_string)
}

fn cc_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(items)) => items.iter().filter_map(|i| i.as_str().map(str::to_string)).collect(),
        Some(Value::String(s)) => {
            s.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
        }
        _ => Vec::new(),
    }
}

impl CliPort {
    pub fn new(exe: PathBuf, operator: &str, backend: Option<String>) -> Self {
        CliPort {
            exe,
            session: format!("station-{operator}"),
            backend,
            pending: Mutex::new(BTreeMap::new()),
            attached: Mutex::new(Vec::new()),
            cleaned: Mutex::new(Default::default()),
        }
    }

    async fn run(&self, args: &[&str], stdin: Option<&str>, timeout: Duration) -> Result<Output, PortError> {
        let mut cmd = tokio::process::Command::new(&self.exe);
        if let Some(backend) = &self.backend {
            cmd.args(["--backend", backend]);
        }
        cmd.args(args)
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| PortError::new(format!("cannot run telex: {e}")))?;
        if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
            pipe.write_all(input.as_bytes())
                .await
                .map_err(|e| PortError::new(format!("writing to telex: {e}")))?;
            drop(pipe);
        }
        let output = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| PortError::uncertain("telex did not respond in time"))?
            .map_err(|e| PortError::uncertain(format!("telex failed: {e}")))?;
        Ok(Output {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }

    fn failure(out: &Output) -> PortError {
        let message = if out.stderr.is_empty() { format!("telex exited {}", out.code) } else { out.stderr.clone() };
        if message.contains("NeedsAttach") || message.contains("explicit attach required") {
            PortError::needs_attach(message)
        } else {
            PortError::new(message)
        }
    }

    async fn json(&self, args: &[&str], stdin: Option<&str>) -> Result<Value, PortError> {
        let out = self.run(args, stdin, Duration::from_secs(20)).await?;
        if out.code != 0 {
            return Err(Self::failure(&out));
        }
        serde_json::from_str(&out.stdout).map_err(|e| PortError::new(format!("unexpected telex output: {e}")))
    }

    fn inbound(&self, m: &Value, address: &str, role: &str, source: &str) -> Option<StoredMessage> {
        Some(StoredMessage {
            id: m.get("id")?.as_i64()?,
            thread_id: m.get("thread_id")?.as_i64()?,
            parent_id: m.get("parent_id").and_then(Value::as_i64),
            direction: Direction::Inbound,
            from: text(m.get("from")).or_else(|| text(m.get("from_addr"))),
            to: text(m.get("primary_to")).or_else(|| text(m.get("to_addr"))).or_else(|| text(m.get("to")))?,
            cc: cc_list(m.get("cc")),
            kind: text(m.get("kind")).unwrap_or_else(|| "note".into()),
            attention: text(m.get("attention")).unwrap_or_else(|| "background".into()),
            requires_disposition: m.get("requires_disposition").and_then(Value::as_bool).unwrap_or(false),
            subject: text(m.get("subject")),
            body: text(m.get("body")).unwrap_or_default(),
            metadata: text(m.get("metadata")),
            sent_at_ms: m.get("sent_at_ms").and_then(Value::as_i64).unwrap_or_else(now_ms),
            ingested_at_ms: now_ms(),
            role: role.into(),
            station_address: address.to_string(),
            delivery: m.get("delivery_id").and_then(Value::as_i64).map(|delivery_id| DeliveryRef {
                logical_store_id: self.store_id(),
                message_id: m.get("id").and_then(Value::as_i64).unwrap_or(0),
                recipient: address.to_string(),
                delivery_id,
            }),
            acked: false,
            disposition: None,
            receipt: None,
            recovery: None,
            source: source.into(),
        })
    }

    async fn directory(&self) -> Result<Value, PortError> {
        self.json(&["address", "list", "--json"], None).await
    }
}

/// Counterpart presence from the CLI directory. Shared with the Application
/// Client port, which has no directory read of its own yet.
pub fn presence_from_directory(value: &Value) -> (BTreeMap<String, Presence>, BTreeMap<String, Option<i64>>) {
    let mut presence = BTreeMap::new();
    let mut pending = BTreeMap::new();
    let now = now_ms();
    for a in value.get("addresses").and_then(Value::as_array).into_iter().flatten() {
        let Some(address) = a.get("address").and_then(Value::as_str) else { continue };
        let occupied = a.get("occupied").and_then(Value::as_bool).unwrap_or(false);
        let station_health = a.get("station_health").and_then(Value::as_str).unwrap_or("");
        let member = a.get("daemon_members").and_then(Value::as_array).and_then(|m| m.first());
        let live_waiters = a.get("live_waiters_count").and_then(Value::as_i64).unwrap_or(0);
        let push = member.and_then(|m| m.get("push_registered")).and_then(Value::as_bool).unwrap_or(false);
        let state = if !occupied {
            "offline"
        } else if matches!(station_health, "armed" | "recently_delivered" | "attended_push") || live_waiters > 0 || push {
            "listening"
        } else {
            "attached"
        };
        let clean = |v: Option<&Value>| {
            v.and_then(Value::as_str).filter(|s| !s.is_empty() && *s != "unknown").map(str::to_string)
        };
        presence.insert(
            address.to_string(),
            Presence {
                state: state.into(),
                detail: clean(a.get("health_detail")),
                host: member.and_then(|m| clean(m.get("host"))),
                description: clean(a.get("description")),
                tags: clean(a.get("tags")),
                checked_at_ms: now,
            },
        );
        pending.insert(address.to_string(), a.get("pending_unconsumed_count").and_then(Value::as_i64));
    }
    (presence, pending)
}

#[async_trait]
impl TelexPort for CliPort {
    fn kind(&self) -> &'static str {
        "cli"
    }

    fn note(&self) -> Option<String> {
        Some(
            "Attending through the telex CLI. On macOS, Telex refuses the Application Client daemon \
             bootstrap, so exact send reconciliation, human-authored reply metadata and “seen” \
             receipts are unavailable in this build."
                .into(),
        )
    }

    fn store_id(&self) -> String {
        match &self.backend {
            Some(backend) => format!("backend:{backend}"),
            None => "backend:default".into(),
        }
    }

    async fn attach(&self, address: &str, description: &str) -> Result<(), PortError> {
        // First attach in this process: a crashed predecessor of this same session
        // can leave a dead anchor pid and an orphaned waiter behind, which would end
        // every new wait with "presence ended". Release that membership cleanly
        // before claiming it again. Queued deliveries are untouched.
        let first = !self.cleaned.lock().unwrap().contains(address);
        if first {
            let _ = self
                .run(&["station", "stop", "--address", address, "--session", &self.session, "--json"], None, Duration::from_secs(20))
                .await;
            self.cleaned.lock().unwrap().insert(address.to_string());
        }
        let anchor = format!("anchor:{}", std::process::id());
        self.json(
            &[
                "attach", "--address", address, "--session", &self.session, "--description", description,
                "--tags", "station,human", "--watch-pid", &anchor, "--json",
            ],
            None,
        )
        .await?;
        let mut attached = self.attached.lock().unwrap();
        if !attached.iter().any(|a| a == address) {
            attached.push(address.to_string());
        }
        Ok(())
    }

    async fn detach(&self, address: &str) -> Result<(), PortError> {
        self.attached.lock().unwrap().retain(|a| a != address);
        let out = self
            .run(&["station", "stop", "--address", address, "--session", &self.session, "--json"], None, Duration::from_secs(20))
            .await?;
        if out.code != 0 {
            return Err(Self::failure(&out));
        }
        Ok(())
    }

    async fn receive(&self, address: &str, timeout_ms: u64) -> Result<Option<StoredMessage>, PortError> {
        let timeout = timeout_ms.to_string();
        let out = self
            .run(
                &["wait", "--address", address, "--session", &self.session, "--timeout-ms", &timeout, "--json"],
                None,
                Duration::from_millis(timeout_ms + 20_000),
            )
            .await?;
        match out.code {
            0 => {
                let value: Value = serde_json::from_str(&out.stdout)
                    .map_err(|e| PortError::new(format!("unexpected wait output: {e}")))?;
                let role = match value.get("delivery_role").and_then(Value::as_str) {
                    Some("to") | None => "primary",
                    Some(_) => "cc",
                };
                self.inbound(&value, address, role, "receive")
                    .map(Some)
                    .ok_or_else(|| PortError::new("wait returned a message without an id"))
            }
            2 => Ok(None),
            // Quarantined delivery: not a message, never acknowledged; keep receiving.
            6 => Ok(None),
            3 | 5 => Err(PortError::needs_attach(format!("wait exited {}: {}", out.code, out.stderr))),
            _ => Err(Self::failure(&out)),
        }
    }

    async fn acknowledge(&self, message: &StoredMessage) -> Result<(), PortError> {
        let id = message.id.to_string();
        self.json(
            &["ack", "--address", &message.station_address, "--session", &self.session, "--id", &id, "--json"],
            None,
        )
        .await
        .map(|_| ())
    }

    async fn history(&self, address: &str, after_id: i64) -> Result<(Vec<StoredMessage>, i64), PortError> {
        let since = after_id.to_string();
        let out = self
            .run(&["export", "--address", address, "--since", &since], None, Duration::from_secs(30))
            .await?;
        if out.code != 0 {
            return Err(Self::failure(&out));
        }
        let mut cursor = after_id;
        let mut rows = Vec::new();
        for line in out.stdout.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(value) = serde_json::from_str::<Value>(line) else { continue };
            let Some(m) = value.get("message") else { continue };
            let from = text(m.get("from_addr"));
            let to = text(m.get("to_addr")).unwrap_or_default();
            let outbound = from.as_deref() == Some(address);
            let role = if outbound {
                "author"
            } else if to == address {
                "primary"
            } else {
                "cc"
            };
            let Some(mut row) = self.inbound(m, address, role, "history") else { continue };
            cursor = cursor.max(row.id);
            let dispositions = value.get("dispositions").and_then(Value::as_array).cloned().unwrap_or_default();
            let latest_by = |recipient: &str| {
                dispositions
                    .iter()
                    .filter(|d| d.get("recipient").and_then(Value::as_str) == Some(recipient))
                    .filter_map(|d| text(d.get("state")))
                    .last()
            };
            if outbound {
                row.direction = Direction::Outbound;
                row.to = to.clone();
                let disposition = latest_by(&to);
                row.receipt = Some(Receipt {
                    occupied_at_send: None,
                    consumed: disposition.is_some(),
                    disposition,
                    refreshed_at_ms: now_ms(),
                });
            } else {
                row.disposition = latest_by(address);
                // A recorded disposition implies the delivery was consumed.
                row.acked = row.disposition.is_some();
            }
            rows.push(row);
        }
        Ok((rows, cursor))
    }

    async fn prepare(&self, _item: &OutboxItem) -> Result<Option<Value>, PortError> {
        Ok(None)
    }

    async fn submit(&self, item: &OutboxItem) -> Result<Sent, PortError> {
        let mut args: Vec<String> = Vec::new();
        match item.reply_to {
            Some(id) => args.extend(["reply".into(), "--to-message".into(), id.to_string()]),
            None => args.extend(["send".into(), "--to".into(), item.to.clone()]),
        }
        args.extend([
            "--address".into(),
            item.sender.clone(),
            "--from".into(),
            item.sender.clone(),
            "--session".into(),
            self.session.clone(),
            "--attention".into(),
            item.attention.clone(),
            "--body-stdin".into(),
            "--json".into(),
        ]);
        if let Some(subject) = &item.subject {
            args.extend(["--subject".into(), subject.clone()]);
        }
        if item.requires_disposition {
            args.push("--requires-disposition".into());
        }
        if item.reply_to.is_none() {
            if let Some(metadata) = &item.metadata {
                args.extend(["--metadata".into(), metadata.clone()]);
            }
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let value = self.json(&refs, Some(&item.body)).await?;
        Ok(Sent {
            message_id: value.get("id").and_then(Value::as_i64).ok_or_else(|| PortError::uncertain("no message id returned"))?,
            thread_id: value.get("thread_id").and_then(Value::as_i64).unwrap_or(0),
            sender: text(value.get("from")).unwrap_or_else(|| item.sender.clone()),
            recipient: text(value.get("to")).unwrap_or_else(|| item.to.clone()),
            occupied: value.get("occupied").and_then(Value::as_bool),
            replayed: false,
        })
    }

    async fn reconcile(&self, _item: &OutboxItem) -> Result<Reconciled, PortError> {
        Ok(Reconciled::Unknown(
            "The CLI has no operation identity to reconcile. Check the thread before retrying.".into(),
        ))
    }

    async fn disposition(&self, message: &StoredMessage, state: &str, note: Option<&str>) -> Result<bool, PortError> {
        let verb = match state {
            "handled" => "handle",
            "deferred" => "defer",
            "rejected" => "reject",
            "closed" => "close",
            "escalated" => "escalate",
            // `telex ack` is transport consumption, not an `acknowledged` disposition.
            _ => return Ok(false),
        };
        let id = message.id.to_string();
        let mut args =
            vec![verb, "--address", &message.station_address, "--session", &self.session, "--id", &id, "--json"];
        if let Some(note) = note {
            args.extend(["--note", note]);
        }
        self.json(&args, None).await.map(|_| true)
    }

    async fn receipt(&self, message: &StoredMessage) -> Result<Receipt, PortError> {
        let id = message.id.to_string();
        let value = self.json(&["read", "--address", &message.station_address, "--id", &id, "--json"], None).await?;
        let disposition = value
            .get("dispositions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|d| d.get("recipient").and_then(Value::as_str) == Some(message.to.as_str()))
            .filter_map(|d| text(d.get("state")))
            .last();
        // Inference: if the recipient has nothing pending, this delivery was consumed.
        let drained = self.pending.lock().unwrap().get(&message.to).copied().flatten() == Some(0);
        let previous = message.receipt.clone().unwrap_or_default();
        Ok(Receipt {
            occupied_at_send: previous.occupied_at_send,
            consumed: previous.consumed || disposition.is_some() || drained,
            disposition,
            refreshed_at_ms: now_ms(),
        })
    }

    async fn health(&self) -> Result<Vec<HealthView>, PortError> {
        let value = self.json(&["station", "status", "--session", &self.session, "--json"], None).await?;
        let attached = self.attached.lock().unwrap().clone();
        let mut out = Vec::new();
        for s in value.get("stations").and_then(Value::as_array).into_iter().flatten() {
            let Some(address) = s.get("address").and_then(Value::as_str) else { continue };
            let health = s.get("station_health").and_then(Value::as_str).unwrap_or("");
            let waiters = s.get("live_waiters_count").and_then(Value::as_i64).unwrap_or(0);
            let mine = attached.iter().any(|a| a == address);
            out.push(HealthView {
                address: address.to_string(),
                registered: mine,
                // Between one-shot waits there is briefly no waiter; our loop re-arms at once.
                receive_ready: mine && (waiters > 0 || matches!(health, "armed" | "recently_delivered" | "unattended" | "unattended_with_backlog")),
                sender_ready: mine,
                degraded: s.get("deaf_warn").and_then(Value::as_bool).unwrap_or(false),
                recovering: false,
                pending_unconsumed: s.get("pending_unconsumed_count").and_then(Value::as_i64).unwrap_or(0),
                inbound_actionable: 0,
                principal: None,
                principal_verification: "unavailable".into(),
                last_error: None,
                last_receive_ms: None,
                evidence: vec![format!("station_health={health}"), format!("live_waiters={waiters}")],
            });
        }
        Ok(out)
    }

    async fn presence(&self) -> Result<BTreeMap<String, Presence>, PortError> {
        let (presence, pending) = presence_from_directory(&self.directory().await?);
        *self.pending.lock().unwrap() = pending;
        Ok(presence)
    }
}
