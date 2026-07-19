use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Notify;

#[derive(Clone, Default)]
pub struct ShutdownSignal {
    requested: Arc<AtomicBool>,
    notified: Arc<Notify>,
}

impl ShutdownSignal {
    pub fn request(&self) {
        if !self.requested.swap(true, Ordering::SeqCst) {
            self.notified.notify_waiters();
            self.notified.notify_one();
        }
    }

    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) {
        loop {
            let notified = self.notified.notified();
            if self.is_requested() {
                return;
            }
            notified.await;
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendRequest {
    pub sender: String,
    pub target: String,
    pub kind: String,
    pub attention: String,
    pub requires_disposition: bool,
    pub subject: String,
    pub body: String,
    pub metadata: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendReceipt {
    pub receipt: String,
    pub id: i64,
    pub thread_id: i64,
    pub to: String,
    #[serde(default)]
    pub from: Option<String>,
}

#[derive(Debug, Clone)]
pub enum SendResult {
    Accepted {
        receipt: SendReceipt,
    },
    NeedsAttach {
        reason: Option<String>,
        message: String,
    },
    Rejected {
        code: String,
        message: String,
    },
}

#[async_trait]
pub trait TelexAdapter: Send + Sync {
    async fn attach(&self, sender: &str, runtime_session_id: &str, watcher_pid: u32) -> Result<()>;
    async fn verify_attached(
        &self,
        sender: &str,
        runtime_session_id: &str,
        watcher_pid: u32,
    ) -> Result<()>;
    async fn strict_send(
        &self,
        runtime_session_id: &str,
        request: &SendRequest,
    ) -> Result<SendResult>;
    async fn detach(&self, sender: &str, runtime_session_id: &str) -> Result<()>;
}

pub struct CliTelexAdapter {
    executable: PathBuf,
}

/// Build the `attach` argv. Extracted so tests can assert the invariants (`--no-session-bind`, a
/// `required` watch-pid predicate, and the occupant tag) without spawning a Telex process.
fn attach_args(sender: &str, runtime_session_id: &str, watcher_pid: u32) -> Vec<String> {
    vec![
        "--address".into(),
        sender.into(),
        "--json".into(),
        "attach".into(),
        "--session".into(),
        runtime_session_id.into(),
        "--no-session-bind".into(),
        "--watch-pid".into(),
        format!("{watcher_pid}:required"),
        "--occupant".into(),
        format!("watcher:{watcher_pid}"),
    ]
}

/// Build the `send` argv. Extracted so tests can assert that every send binds an explicit
/// `--session`/`--from` and streams the body over stdin.
fn send_args(runtime_session_id: &str, request: &SendRequest) -> Vec<String> {
    let mut args = vec![
        "--json".into(),
        "send".into(),
        "--session".into(),
        runtime_session_id.into(),
        "--from".into(),
        request.sender.clone(),
        "--to".into(),
        request.target.clone(),
        "--kind".into(),
        request.kind.clone(),
        "--attention".into(),
        request.attention.clone(),
        "--subject".into(),
        request.subject.clone(),
        "--metadata".into(),
        request.metadata.clone(),
        "--body-stdin".into(),
    ];
    if request.requires_disposition {
        args.push("--requires-disposition".into());
    }
    args
}

/// Assert that `status` JSON proves `sender` is attached under exactly this runtime with a live,
/// required predicate for `watcher_pid`. Extracted from `verify_attached` so the multi-sender
/// verification rules can be unit tested against synthetic status payloads.
fn evaluate_status(
    value: &Value,
    sender: &str,
    runtime_session_id: &str,
    watcher_pid: u32,
) -> Result<()> {
    let members = value
        .get("daemon_members")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Telex status omitted daemon_members"))?;
    if members.iter().any(|member| {
        member.get("address").and_then(Value::as_str) == Some(sender)
            && member.get("session_id").and_then(Value::as_str) != Some(runtime_session_id)
            && member.get("idle").and_then(Value::as_bool) == Some(false)
    }) {
        bail!("sender {sender:?} is also attached under another live Telex session");
    }
    let member = members
        .iter()
        .find(|member| {
            member.get("session_id").and_then(Value::as_str) == Some(runtime_session_id)
                && member.get("address").and_then(Value::as_str) == Some(sender)
                && member.get("idle").and_then(Value::as_bool) == Some(false)
        })
        .ok_or_else(|| {
            anyhow!("sender {sender:?} is not attached under runtime {runtime_session_id:?}")
        })?;
    let watch_pids = member
        .get("watch_pids")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("sender {sender:?} status omitted watch_pids"))?;
    let required_pid_present = watch_pids.iter().any(|watch| {
        watch.get("pid").and_then(Value::as_u64) == Some(watcher_pid as u64)
            && watch.get("role").and_then(Value::as_str) == Some("required")
            && watch.get("alive").and_then(Value::as_bool) == Some(true)
            && watch.get("start_time").and_then(Value::as_u64).is_some()
    });
    if !required_pid_present {
        bail!(
            "sender {sender:?} is attached without a live required predicate for watcher PID {watcher_pid}"
        );
    }
    Ok(())
}

impl CliTelexAdapter {
    pub fn new(executable: PathBuf) -> Self {
        Self { executable }
    }

    async fn run(
        &self,
        args: &[String],
        stdin: Option<&str>,
        env: Option<(&str, &str)>,
    ) -> Result<std::process::Output> {
        let mut command = Command::new(&self.executable);
        command.args(args).stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some((key, value)) = env {
            command.env(key, value);
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("start Telex executable {}", self.executable.display()))?;
        if let Some(input) = stdin {
            let mut child_stdin = child
                .stdin
                .take()
                .ok_or_else(|| anyhow!("Telex stdin was not piped"))?;
            child_stdin
                .write_all(input.as_bytes())
                .await
                .context("write Telex send body")?;
            child_stdin
                .shutdown()
                .await
                .context("close Telex send body")?;
        }
        child
            .wait_with_output()
            .await
            .context("wait for Telex subprocess")
    }

    fn command_error(&self, action: &str, output: &std::process::Output) -> anyhow::Error {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow!(
            "telex {action} failed with {}: {}",
            output.status,
            stderr.trim()
        )
    }
}

#[async_trait]
impl TelexAdapter for CliTelexAdapter {
    async fn attach(&self, sender: &str, runtime_session_id: &str, watcher_pid: u32) -> Result<()> {
        let args = attach_args(sender, runtime_session_id, watcher_pid);
        let output = self.run(&args, None, None).await?;
        if !output.status.success() {
            return Err(self.command_error("attach", &output));
        }
        Ok(())
    }

    async fn verify_attached(
        &self,
        sender: &str,
        runtime_session_id: &str,
        watcher_pid: u32,
    ) -> Result<()> {
        let args = vec![
            "--address".into(),
            sender.into(),
            "--json".into(),
            "status".into(),
        ];
        let output = self.run(&args, None, None).await?;
        if !output.status.success() {
            return Err(self.command_error("status", &output));
        }
        let value: Value =
            serde_json::from_slice(&output.stdout).context("Telex status did not return JSON")?;
        evaluate_status(&value, sender, runtime_session_id, watcher_pid)
    }

    async fn strict_send(
        &self,
        runtime_session_id: &str,
        request: &SendRequest,
    ) -> Result<SendResult> {
        let args = send_args(runtime_session_id, request);
        let output = self
            .run(
                &args,
                Some(&request.body),
                Some(("TELEX_WATCHER_INTERNAL_SEND_ONCE_V1", runtime_session_id)),
            )
            .await?;
        if !output.status.success() {
            return Err(self.command_error("strict send", &output));
        }
        let raw: Value = serde_json::from_slice(&output.stdout)
            .context("private Telex send response was not JSON")?;
        match raw.get("type").and_then(Value::as_str) {
            Some("sent") => {
                let receipt: SendReceipt = serde_json::from_value(
                    raw.get("receipt")
                        .cloned()
                        .ok_or_else(|| anyhow!("private Telex send omitted receipt"))?,
                )
                .context("private Telex send receipt was malformed")?;
                Ok(SendResult::Accepted { receipt })
            }
            Some("error") if raw.get("code").and_then(Value::as_str) == Some("NeedsAttach") => {
                Ok(SendResult::NeedsAttach {
                    reason: raw
                        .get("needs_attach_reason")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    message: raw
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("Telex membership needs attachment")
                        .to_owned(),
                })
            }
            Some("error") => Ok(SendResult::Rejected {
                code: raw
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown")
                    .to_owned(),
                message: raw
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Telex rejected send")
                    .to_owned(),
            }),
            _ => bail!("private Telex send returned unexpected response {raw}"),
        }
    }

    async fn detach(&self, sender: &str, runtime_session_id: &str) -> Result<()> {
        let args = vec![
            "--address".into(),
            sender.into(),
            "--json".into(),
            "detach".into(),
            "--session".into(),
            runtime_session_id.into(),
        ];
        let output = self.run(&args, None, None).await?;
        if !output.status.success() {
            return Err(self.command_error("detach", &output));
        }
        Ok(())
    }
}

pub struct LifecycleCoordinator<A> {
    adapter: A,
    runtime_session_id: String,
    watcher_pid: u32,
    attached: BTreeSet<String>,
    shutting_down: bool,
    shutdown_signal: ShutdownSignal,
    generation: u64,
}

impl<A: TelexAdapter> LifecycleCoordinator<A> {
    pub fn new(adapter: A, runtime_session_id: String, watcher_pid: u32) -> Self {
        Self {
            adapter,
            runtime_session_id,
            watcher_pid,
            attached: BTreeSet::new(),
            shutting_down: false,
            shutdown_signal: ShutdownSignal::default(),
            generation: 0,
        }
    }

    pub fn attached(&self) -> &BTreeSet<String> {
        &self.attached
    }

    pub fn shutdown_signal(&self) -> ShutdownSignal {
        self.shutdown_signal.clone()
    }

    fn is_shutting_down(&self) -> bool {
        self.shutting_down || self.shutdown_signal.is_requested()
    }

    pub async fn reconcile(&mut self, desired: &BTreeSet<String>) -> Result<()> {
        if self.is_shutting_down() {
            bail!("runtime is shutting down; sender reconciliation is disabled");
        }
        self.generation = self.generation.saturating_add(1);
        for sender in desired {
            if self.is_shutting_down() {
                bail!("runtime is shutting down; sender reconciliation is disabled");
            }
            self.adapter
                .attach(sender, &self.runtime_session_id, self.watcher_pid)
                .await
                .with_context(|| format!("attach sender {sender:?}"))?;
            // Attach changes Telex state even if the subsequent status query fails. Keep it
            // immediately so bounded shutdown can compensate for every partial reconcile.
            self.attached.insert(sender.clone());
            if self.is_shutting_down() {
                bail!("runtime is shutting down; sender reconciliation is disabled");
            }
            self.adapter
                .verify_attached(sender, &self.runtime_session_id, self.watcher_pid)
                .await
                .with_context(|| format!("verify sender {sender:?}"))?;
        }
        let obsolete: Vec<String> = self.attached.difference(desired).cloned().collect();
        for sender in obsolete {
            if self.is_shutting_down() {
                bail!("runtime is shutting down; sender reconciliation is disabled");
            }
            self.adapter
                .detach(&sender, &self.runtime_session_id)
                .await
                .with_context(|| format!("detach removed sender {sender:?}"))?;
            self.attached.remove(&sender);
        }
        Ok(())
    }

    pub async fn send(
        &mut self,
        desired: &BTreeSet<String>,
        request: &SendRequest,
    ) -> Result<SendReceipt> {
        if self.is_shutting_down() {
            bail!("runtime is shutting down; sends are disabled");
        }
        match self
            .adapter
            .strict_send(&self.runtime_session_id, request)
            .await?
        {
            SendResult::Accepted { receipt } => Ok(receipt),
            SendResult::NeedsAttach { reason, message }
                if reason.as_deref() != Some("deliberately_detached") =>
            {
                if self.is_shutting_down() {
                    bail!("runtime is shutting down; sends are disabled");
                }
                self.reconcile(desired)
                    .await
                    .context("reconcile after NeedsAttach")?;
                if self.is_shutting_down() {
                    bail!("runtime is shutting down; sends are disabled");
                }
                match self
                    .adapter
                    .strict_send(&self.runtime_session_id, request)
                    .await?
                {
                    SendResult::Accepted { receipt } => Ok(receipt),
                    SendResult::NeedsAttach { reason, message } => bail!(
                        "Telex membership remained unavailable after one reconcile: {message} ({})",
                        reason.unwrap_or_else(|| "unspecified".to_string())
                    ),
                    SendResult::Rejected { code, message } => {
                        bail!("Telex rejected send after reconcile: {code}: {message}")
                    }
                }
            }
            SendResult::NeedsAttach { reason, message } => bail!(
                "Telex sender was deliberately detached: {message} ({})",
                reason.unwrap_or_else(|| "unspecified".to_string())
            ),
            SendResult::Rejected { code, message } => {
                bail!("Telex rejected send: {code}: {message}")
            }
        }
    }

    pub async fn shutdown(&mut self) -> Vec<(String, String)> {
        self.shutdown_signal.request();
        self.shutting_down = true;
        let senders: Vec<String> = self.attached.iter().cloned().collect();
        let mut outcomes = Vec::new();
        for sender in senders {
            let mut last_error = None;
            for _ in 0..3 {
                match self.adapter.detach(&sender, &self.runtime_session_id).await {
                    Ok(()) => {
                        last_error = None;
                        break;
                    }
                    Err(error) => {
                        last_error = Some(error.to_string());
                    }
                }
            }
            outcomes.push((sender, last_error.unwrap_or_else(|| "detached".to_string())));
        }
        self.attached.clear();
        outcomes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct FakeAdapter {
        sent: Arc<Mutex<Vec<String>>>,
        attach_count: Arc<Mutex<u32>>,
        needs_attach_once: Arc<Mutex<bool>>,
        fail_attach_for: Arc<Mutex<Option<String>>>,
        fail_detach_for: Arc<Mutex<Option<String>>>,
        detached: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl TelexAdapter for FakeAdapter {
        async fn attach(&self, sender: &str, _: &str, _: u32) -> Result<()> {
            *self.attach_count.lock().unwrap() += 1;
            if self.fail_attach_for.lock().unwrap().as_deref() == Some(sender) {
                bail!("forced attach failure for {sender}");
            }
            Ok(())
        }
        async fn verify_attached(&self, _: &str, _: &str, _: u32) -> Result<()> {
            Ok(())
        }
        async fn strict_send(&self, _: &str, request: &SendRequest) -> Result<SendResult> {
            if *self.needs_attach_once.lock().unwrap() {
                *self.needs_attach_once.lock().unwrap() = false;
                return Ok(SendResult::NeedsAttach {
                    reason: Some("restart_lost".into()),
                    message: "restart".into(),
                });
            }
            self.sent.lock().unwrap().push(request.body.clone());
            Ok(SendResult::Accepted {
                receipt: SendReceipt {
                    receipt: "delivered".into(),
                    id: 1,
                    thread_id: 1,
                    to: request.target.clone(),
                    from: Some(request.sender.clone()),
                },
            })
        }
        async fn detach(&self, sender: &str, _: &str) -> Result<()> {
            self.detached.lock().unwrap().push(sender.to_string());
            if self.fail_detach_for.lock().unwrap().as_deref() == Some(sender) {
                bail!("forced detach failure for {sender}");
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn needs_attach_reconciles_once_before_retrying_send() {
        let fake = FakeAdapter::default();
        *fake.needs_attach_once.lock().unwrap() = true;
        let probe = fake.clone();
        let mut coordinator = LifecycleCoordinator::new(fake, "runtime".into(), 10);
        let desired = BTreeSet::from(["service:watcher".to_string()]);
        coordinator.reconcile(&desired).await.unwrap();
        coordinator
            .send(
                &desired,
                &SendRequest {
                    sender: "service:watcher".into(),
                    target: "target".into(),
                    kind: "watch.test".into(),
                    attention: "background".into(),
                    requires_disposition: false,
                    subject: "subject".into(),
                    body: "body".into(),
                    metadata: "{}".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(probe.sent.lock().unwrap().as_slice(), ["body"]);
        assert_eq!(*probe.attach_count.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn partial_reconcile_is_tracked_and_shutdown_detaches_it() {
        let fake = FakeAdapter::default();
        *fake.fail_attach_for.lock().unwrap() = Some("sender-b".into());
        let probe = fake.clone();
        let mut coordinator = LifecycleCoordinator::new(fake, "runtime".into(), 10);
        let desired = BTreeSet::from(["sender-a".to_string(), "sender-b".to_string()]);

        assert!(coordinator.reconcile(&desired).await.is_err());
        assert_eq!(
            coordinator.attached(),
            &BTreeSet::from(["sender-a".to_string()])
        );

        assert_eq!(
            coordinator.shutdown().await,
            vec![("sender-a".to_string(), "detached".to_string())]
        );
        assert_eq!(probe.detached.lock().unwrap().as_slice(), ["sender-a"]);
    }

    #[tokio::test]
    async fn partial_reconcile_shutdown_bounds_detach_retries() {
        let fake = FakeAdapter::default();
        *fake.fail_attach_for.lock().unwrap() = Some("sender-b".into());
        *fake.fail_detach_for.lock().unwrap() = Some("sender-a".into());
        let probe = fake.clone();
        let mut coordinator = LifecycleCoordinator::new(fake, "runtime".into(), 10);
        let desired = BTreeSet::from(["sender-a".to_string(), "sender-b".to_string()]);

        assert!(coordinator.reconcile(&desired).await.is_err());
        let outcomes = coordinator.shutdown().await;

        assert_eq!(
            probe.detached.lock().unwrap().as_slice(),
            [
                "sender-a".to_string(),
                "sender-a".to_string(),
                "sender-a".to_string()
            ]
        );
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].1.contains("forced detach failure"));
    }

    #[tokio::test]
    async fn shutdown_request_prevents_reconcile_from_reattaching() {
        let fake = FakeAdapter::default();
        let probe = fake.clone();
        let mut coordinator = LifecycleCoordinator::new(fake, "runtime".into(), 10);
        let shutdown = coordinator.shutdown_signal();
        shutdown.request();

        assert!(coordinator
            .reconcile(&BTreeSet::from(["sender".to_string()]))
            .await
            .is_err());
        assert_eq!(*probe.attach_count.lock().unwrap(), 0);
    }

    fn sample_request() -> SendRequest {
        SendRequest {
            sender: "service:watcher".into(),
            target: "target".into(),
            kind: "watch.test".into(),
            attention: "background".into(),
            requires_disposition: false,
            subject: "subject".into(),
            body: "body".into(),
            metadata: "{}".into(),
        }
    }

    #[test]
    fn attach_args_bind_session_without_session_bind_and_required_predicate() {
        let args = attach_args("service:watcher", "runtime-7", 4321);
        assert!(args.contains(&"--no-session-bind".to_string()));
        // --session must be immediately followed by the runtime id.
        let session_idx = args.iter().position(|a| a == "--session").unwrap();
        assert_eq!(args[session_idx + 1], "runtime-7");
        let pid_idx = args.iter().position(|a| a == "--watch-pid").unwrap();
        assert_eq!(args[pid_idx + 1], "4321:required");
        let occupant_idx = args.iter().position(|a| a == "--occupant").unwrap();
        assert_eq!(args[occupant_idx + 1], "watcher:4321");
    }

    #[test]
    fn send_args_always_bind_explicit_session_and_from_over_stdin() {
        let mut request = sample_request();
        let args = send_args("runtime-9", &request);
        let session_idx = args.iter().position(|a| a == "--session").unwrap();
        assert_eq!(args[session_idx + 1], "runtime-9");
        let from_idx = args.iter().position(|a| a == "--from").unwrap();
        assert_eq!(args[from_idx + 1], "service:watcher");
        assert!(args.contains(&"--body-stdin".to_string()));
        assert!(!args.contains(&"--requires-disposition".to_string()));

        request.requires_disposition = true;
        let args = send_args("runtime-9", &request);
        assert!(args.contains(&"--requires-disposition".to_string()));
    }

    fn status_with(member: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "daemon_members": [member] })
    }

    fn live_member() -> serde_json::Value {
        serde_json::json!({
            "address": "service:watcher",
            "session_id": "runtime-1",
            "idle": false,
            "watch_pids": [
                { "pid": 100, "role": "required", "alive": true, "start_time": 12345 }
            ],
        })
    }

    #[test]
    fn evaluate_status_accepts_live_required_predicate() {
        let value = status_with(live_member());
        assert!(evaluate_status(&value, "service:watcher", "runtime-1", 100).is_ok());
    }

    #[test]
    fn evaluate_status_rejects_membership_under_another_session() {
        let mut member = live_member();
        member["session_id"] = serde_json::json!("someone-else");
        let value = status_with(member);
        let error = evaluate_status(&value, "service:watcher", "runtime-1", 100)
            .unwrap_err()
            .to_string();
        assert!(error.contains("another live Telex session"), "{error}");
    }

    #[test]
    fn evaluate_status_rejects_wrong_pid_role_or_dead_predicate() {
        // Wrong pid.
        let mut member = live_member();
        member["watch_pids"] = serde_json::json!([
            { "pid": 999, "role": "required", "alive": true, "start_time": 1 }
        ]);
        assert!(
            evaluate_status(&status_with(member), "service:watcher", "runtime-1", 100).is_err()
        );

        // Non-required role.
        let mut member = live_member();
        member["watch_pids"] = serde_json::json!([
            { "pid": 100, "role": "optional", "alive": true, "start_time": 1 }
        ]);
        assert!(
            evaluate_status(&status_with(member), "service:watcher", "runtime-1", 100).is_err()
        );

        // Dead predicate.
        let mut member = live_member();
        member["watch_pids"] = serde_json::json!([
            { "pid": 100, "role": "required", "alive": false, "start_time": 1 }
        ]);
        assert!(
            evaluate_status(&status_with(member), "service:watcher", "runtime-1", 100).is_err()
        );

        // Missing start_time (never actually verified alive).
        let mut member = live_member();
        member["watch_pids"] = serde_json::json!([
            { "pid": 100, "role": "required", "alive": true }
        ]);
        assert!(
            evaluate_status(&status_with(member), "service:watcher", "runtime-1", 100).is_err()
        );
    }

    #[test]
    fn evaluate_status_rejects_empty_watch_pids() {
        let mut member = live_member();
        member["watch_pids"] = serde_json::json!([]);
        assert!(
            evaluate_status(&status_with(member), "service:watcher", "runtime-1", 100).is_err()
        );
    }

    #[tokio::test]
    async fn deliberately_detached_send_fails_without_reconcile() {
        #[derive(Clone, Default)]
        struct DetachedAdapter {
            attach_count: Arc<Mutex<u32>>,
        }
        #[async_trait]
        impl TelexAdapter for DetachedAdapter {
            async fn attach(&self, _: &str, _: &str, _: u32) -> Result<()> {
                *self.attach_count.lock().unwrap() += 1;
                Ok(())
            }
            async fn verify_attached(&self, _: &str, _: &str, _: u32) -> Result<()> {
                Ok(())
            }
            async fn strict_send(&self, _: &str, _: &SendRequest) -> Result<SendResult> {
                Ok(SendResult::NeedsAttach {
                    reason: Some("deliberately_detached".into()),
                    message: "operator detached".into(),
                })
            }
            async fn detach(&self, _: &str, _: &str) -> Result<()> {
                Ok(())
            }
        }

        let fake = DetachedAdapter::default();
        let probe = fake.clone();
        let mut coordinator = LifecycleCoordinator::new(fake, "runtime".into(), 10);
        let desired = BTreeSet::from(["service:watcher".to_string()]);
        let error = coordinator
            .send(&desired, &sample_request())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("deliberately detached"), "{error}");
        // A deliberate detach must never trigger reconciliation/reattach.
        assert_eq!(*probe.attach_count.lock().unwrap(), 0);
    }
}
