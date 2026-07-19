use crate::config::RuntimeConfig;
use crate::model::{
    AddressOccupancy, DispositionRecord, SentReceipt, StationMessage, ThreadEntry, ThreadView,
    HUMAN_REPLY_KIND,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::watch;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const EXPORT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct TelexCli {
    config: Arc<RuntimeConfig>,
    session_id: String,
}

#[derive(Clone, Debug)]
struct CommandSpec {
    args: Vec<OsString>,
    env: BTreeMap<String, OsString>,
    stdin: Option<Vec<u8>>,
}

#[derive(Debug)]
pub struct RunningWait {
    pid: Option<u32>,
    child: Child,
    stdout: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
    stderr: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
}

#[derive(Debug)]
pub enum WaitExecution {
    Exited {
        code: i32,
        stdout: String,
        stderr: String,
    },
    Shutdown,
}

#[allow(dead_code)] // Optional wait fields are parsed to enforce the CLI contract before enrichment.
#[derive(Clone, Debug, Deserialize)]
pub struct WaitPayload {
    pub id: i64,
    pub thread_id: i64,
    pub parent_id: Option<i64>,
    pub from: Option<String>,
    pub to: String,
    pub delivered_to: String,
    pub primary_to: String,
    #[serde(default)]
    pub cc: Option<Vec<String>>,
    pub delivery_role: String,
    pub kind: String,
    pub attention: String,
    pub requires_disposition: bool,
    pub requires_disposition_for_current_recipient: bool,
    pub subject: Option<String>,
    pub body: String,
    pub sent_at_ms: i64,
    pub buffered_at_ms: i64,
    pub lease_epoch: i64,
    pub waiter_exit_ms: i64,
}

#[derive(Clone, Debug, Deserialize)]
struct MessageWire {
    id: i64,
    thread_id: i64,
    parent_id: Option<i64>,
    from_addr: Option<String>,
    to_addr: String,
    cc: Option<String>,
    kind: String,
    attention: String,
    requires_disposition: bool,
    subject: Option<String>,
    body: String,
    metadata: Option<String>,
    sent_at_ms: i64,
    created_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize)]
struct DispositionWire {
    id: i64,
    message_id: i64,
    recipient: String,
    state: String,
    note: Option<String>,
    by_principal: Option<String>,
    at_ms: i64,
}

#[derive(Clone, Debug, Deserialize)]
struct ExportItemWire {
    message: MessageWire,
    dispositions: Vec<DispositionWire>,
}

#[derive(Clone, Debug, Deserialize)]
struct InboxResponseWire {
    address: String,
    count: usize,
    items: Vec<InboxItemWire>,
}

#[derive(Clone, Debug, Deserialize)]
struct InboxItemWire {
    #[serde(flatten)]
    message: MessageWire,
    delivered_to: String,
    primary_to: String,
    cc_recipients: Vec<String>,
    delivery_role: String,
    requires_disposition_for_current_recipient: bool,
    latest_disposition: Option<String>,
    actionable: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct DeliveryWire {
    delivered_to: String,
    primary_to: String,
    cc: Vec<String>,
    delivery_role: String,
    requires_disposition_for_current_recipient: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct ReadResponseWire {
    message: MessageWire,
    dispositions: Vec<DispositionWire>,
    delivery: DeliveryWire,
    thread: Vec<ExportItemWire>,
}

#[derive(Debug, Deserialize)]
struct AttachWire {
    address: String,
    session_id: String,
    lease_epoch: i64,
}

#[derive(Debug, Deserialize)]
struct AckWire {
    state: String,
    message_id: i64,
    recipient: String,
}

#[derive(Debug, Deserialize)]
struct ReceiptWire {
    receipt: String,
    id: i64,
    thread_id: i64,
    parent_id: Option<i64>,
    to: String,
    from: Option<String>,
    attention: Option<String>,
    requires_disposition: Option<bool>,
    occupied: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct OccupancyWire {
    occupied: bool,
    age_secs: f64,
    occupant: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StatusWire {
    address: String,
    occupancy: OccupancyWire,
    station_health: Option<String>,
    pending_unconsumed_count: Option<i64>,
    live_waiters_count: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct StationStatusWire {
    count: usize,
    stations: Vec<StationStatusItemWire>,
}

#[derive(Debug, Deserialize)]
struct StationStatusItemWire {
    address: String,
    station_health: String,
    pending_unconsumed_count: i64,
    live_waiters_count: i64,
}

#[derive(Debug, Deserialize)]
struct VersionRootWire {
    version: VersionWire,
}

#[derive(Debug, Deserialize)]
struct VersionWire {
    package_version: String,
    build_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StopWire {
    address: String,
    session_id: String,
    detached: bool,
    waiters_after: usize,
}

impl TelexCli {
    pub fn new(config: Arc<RuntimeConfig>, session_id: String) -> Self {
        Self { config, session_id }
    }

    pub async fn version(&self) -> Result<String, String> {
        let parsed: VersionRootWire = self
            .run_json(self.spec(&["version"], &self.config.station_address, true))
            .await?;
        if parsed.version.package_version.trim().is_empty() {
            return Err("telex version response contains an empty required field".into());
        }
        Ok(
            match parsed
                .version
                .build_id
                .filter(|build_id| !build_id.trim().is_empty())
            {
                Some(build_id) => format!("{} (build {build_id})", parsed.version.package_version),
                None => parsed.version.package_version,
            },
        )
    }

    pub async fn attach(&self, anchor_pid: u32) -> Result<(), String> {
        let watch_pid = format!("anchor:{anchor_pid}");
        let spec = self.spec_owned(
            vec![
                "attach".into(),
                "--session".into(),
                self.session_id.clone().into(),
                "--watch-pid".into(),
                watch_pid.into(),
                "--description".into(),
                "Telex Operator Station spike".into(),
            ],
            &self.config.station_address,
            true,
            None,
        );
        let parsed: AttachWire = self.run_json(spec).await?;
        if parsed.address != self.config.station_address
            || parsed.session_id != self.session_id
            || parsed.lease_epoch < 1
        {
            return Err("telex attach response does not match the Station identity".into());
        }
        Ok(())
    }

    pub async fn export_history(&self) -> Result<Vec<StationMessage>, String> {
        let spec = self.spec(
            &[
                "export",
                "--address",
                &self.config.station_address,
                "--since",
                "0",
            ],
            &self.config.station_address,
            true,
        );
        let items: Vec<ExportItemWire> = self.run_json_lines(spec).await?;
        Ok(items
            .into_iter()
            .map(|item| {
                export_message(
                    item,
                    &self.config.station_address,
                    &self.config.store_fingerprint,
                )
            })
            .collect())
    }

    pub async fn inbox(&self) -> Result<Vec<StationMessage>, String> {
        let parsed: InboxResponseWire = self
            .run_json(self.spec(
                &["inbox", "--all", "--limit", "200"],
                &self.config.station_address,
                true,
            ))
            .await?;
        if parsed.address != self.config.station_address || parsed.count != parsed.items.len() {
            return Err("telex inbox response address/count does not match its payload".into());
        }
        Ok(parsed
            .items
            .into_iter()
            .map(|item| inbox_message(item, &self.config.store_fingerprint))
            .collect())
    }

    pub async fn read_full(&self, message_id: i64) -> Result<ThreadView, String> {
        let id = message_id.to_string();
        let parsed: ReadResponseWire = self
            .run_json(self.spec(
                &["read", "--id", &id, "--full"],
                &self.config.station_address,
                true,
            ))
            .await?;
        if parsed.message.id != message_id {
            return Err("telex read response returned a different message id".into());
        }
        let latest = latest_for(&parsed.dispositions, &self.config.station_address);
        let actionable = parsed.delivery.requires_disposition_for_current_recipient
            && !latest.as_deref().is_some_and(is_terminal);
        let mut message = convert_message(
            parsed.message,
            parsed.delivery.delivered_to,
            parsed.delivery.primary_to,
            parsed.delivery.cc,
            parsed.delivery.delivery_role,
            parsed.delivery.requires_disposition_for_current_recipient,
            latest,
            actionable,
            &self.config.store_fingerprint,
        );
        message.ack_pending = false;
        let dispositions = parsed
            .dispositions
            .into_iter()
            .map(DispositionRecord::from)
            .collect();
        let thread = parsed
            .thread
            .into_iter()
            .map(|item| {
                let message = export_message(
                    item.clone(),
                    &self.config.station_address,
                    &self.config.store_fingerprint,
                );
                ThreadEntry {
                    message,
                    dispositions: item
                        .dispositions
                        .into_iter()
                        .map(DispositionRecord::from)
                        .collect(),
                }
            })
            .collect();
        Ok(ThreadView {
            message,
            dispositions,
            thread,
        })
    }

    pub fn parse_wait_payload(&self, stdout: &str) -> Result<WaitPayload, String> {
        parse_json(stdout, "wait")
    }

    pub fn spawn_wait(&self) -> Result<RunningWait, String> {
        let spec = self.spec_owned(
            vec![
                "wait".into(),
                "--session".into(),
                self.session_id.clone().into(),
                "--timeout-ms".into(),
                "30000".into(),
            ],
            &self.config.station_address,
            true,
            None,
        );
        let mut child = self.spawn(spec)?;
        let pid = child.id();
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to capture telex wait stdout".to_string())?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| "failed to capture telex wait stderr".to_string())?;
        Ok(RunningWait {
            pid,
            child,
            stdout: tokio::spawn(async move {
                let mut bytes = Vec::new();
                stdout.read_to_end(&mut bytes).await?;
                Ok(bytes)
            }),
            stderr: tokio::spawn(async move {
                let mut bytes = Vec::new();
                stderr.read_to_end(&mut bytes).await?;
                Ok(bytes)
            }),
        })
    }

    pub async fn ack(&self, message_id: i64, timeout: Duration) -> Result<(), String> {
        let id = message_id.to_string();
        let spec = self.spec_owned(
            vec![
                "ack".into(),
                "--id".into(),
                id.into(),
                "--recipient".into(),
                self.config.station_address.clone().into(),
                "--session".into(),
                self.session_id.clone().into(),
            ],
            &self.config.station_address,
            true,
            None,
        );
        let parsed: AckWire = self.run_json_timeout(spec, timeout).await?;
        if parsed.state != "acknowledged"
            || parsed.message_id != message_id
            || parsed.recipient != self.config.station_address
        {
            return Err("telex ack response does not match the delivery".into());
        }
        Ok(())
    }

    pub async fn reply(&self, message_id: i64, body: String) -> Result<SentReceipt, String> {
        if body.trim().is_empty() {
            return Err("reply body cannot be empty".into());
        }
        let spec = self.spec_owned(
            vec![
                "reply".into(),
                "--to-message".into(),
                message_id.to_string().into(),
                "--body-file".into(),
                "-".into(),
                "--kind".into(),
                HUMAN_REPLY_KIND.into(),
                "--attention".into(),
                "background".into(),
                "--session".into(),
                self.session_id.clone().into(),
            ],
            &self.config.station_address,
            true,
            Some(body.into_bytes()),
        );
        let parsed: ReceiptWire = self.run_json(spec).await?;
        Ok(parsed.into())
    }

    pub async fn disposition(
        &self,
        message_id: i64,
        disposition: &str,
        note: Option<String>,
    ) -> Result<DispositionRecord, String> {
        if !matches!(disposition, "defer" | "handle" | "close") {
            return Err("disposition must be defer, handle, or close".into());
        }
        let mut args = vec![
            disposition.into(),
            "--id".into(),
            message_id.to_string().into(),
            "--recipient".into(),
            self.config.station_address.clone().into(),
            "--session".into(),
            self.session_id.clone().into(),
        ];
        if let Some(note) = note.filter(|note| !note.trim().is_empty()) {
            args.push("--note".into());
            args.push(note.into());
        }
        let parsed: DispositionWire = self
            .run_json(self.spec_owned(args, &self.config.station_address, true, None))
            .await?;
        if parsed.message_id != message_id || parsed.recipient != self.config.station_address {
            return Err("telex disposition response does not match the requested message".into());
        }
        Ok(parsed.into())
    }

    pub async fn address_status(&self, address: &str) -> Result<AddressOccupancy, String> {
        let parsed: StatusWire = self.run_json(self.spec(&["status"], address, true)).await?;
        if parsed.address != address {
            return Err("telex status response returned a different address".into());
        }
        Ok(AddressOccupancy {
            address: address.to_string(),
            occupied: parsed.occupancy.occupied,
            age_secs: parsed.occupancy.age_secs,
            occupant: parsed.occupancy.occupant,
            station_health: parsed.station_health,
            pending_unconsumed_count: parsed.pending_unconsumed_count,
            live_waiters_count: parsed.live_waiters_count,
            error: None,
            refreshed_at_ms: now_ms(),
        })
    }

    pub async fn station_status(&self) -> Result<AddressOccupancy, String> {
        let spec = self.spec_owned(
            vec![
                "station".into(),
                "status".into(),
                "--session".into(),
                self.session_id.clone().into(),
            ],
            &self.config.station_address,
            true,
            None,
        );
        let parsed: StationStatusWire = self.run_json(spec).await?;
        if parsed.count != parsed.stations.len() {
            return Err("telex station status count does not match its payload".into());
        }
        let station = parsed
            .stations
            .into_iter()
            .find(|station| station.address == self.config.station_address);
        Ok(match station {
            Some(station) => AddressOccupancy {
                address: station.address,
                occupied: true,
                age_secs: 0.0,
                occupant: None,
                station_health: Some(station.station_health),
                pending_unconsumed_count: Some(station.pending_unconsumed_count),
                live_waiters_count: Some(station.live_waiters_count),
                error: None,
                refreshed_at_ms: now_ms(),
            },
            None => {
                return Err(format!(
                    "Station {} is not attached for session {}",
                    self.config.station_address, self.session_id
                ))
            }
        })
    }

    pub async fn station_stop(&self) -> Result<(), String> {
        let spec = self.spec_owned(
            vec![
                "station".into(),
                "stop".into(),
                "--session".into(),
                self.session_id.clone().into(),
                "--wait-grace-ms".into(),
                "3000".into(),
            ],
            &self.config.station_address,
            true,
            None,
        );
        let parsed: StopWire = self.run_json(spec).await?;
        if parsed.address != self.config.station_address
            || parsed.session_id != self.session_id
            || parsed.waiters_after != 0
        {
            return Err("telex station stop did not fully release this Station".into());
        }
        let _was_detached = parsed.detached;
        Ok(())
    }

    fn spec(&self, args: &[&str], address: &str, json: bool) -> CommandSpec {
        self.spec_owned(
            args.iter().map(|value| OsString::from(*value)).collect(),
            address,
            json,
            None,
        )
    }

    fn spec_owned(
        &self,
        mut verb_args: Vec<OsString>,
        address: &str,
        json: bool,
        stdin: Option<Vec<u8>>,
    ) -> CommandSpec {
        let mut args = vec![
            "--db".into(),
            self.config.database_path.as_os_str().to_os_string(),
            "--address".into(),
            address.into(),
        ];
        if json {
            args.push("--json".into());
        }
        args.append(&mut verb_args);
        let env = BTreeMap::from([
            (
                "TELEX_SESSION_ID".to_string(),
                self.session_id.clone().into(),
            ),
            ("TELEX_ADDRESS".to_string(), address.into()),
            (
                "TELEX_OPERATOR_SPIKE_DB".to_string(),
                self.config.database_path.as_os_str().to_os_string(),
            ),
        ]);
        CommandSpec { args, env, stdin }
    }

    async fn run_json<T: DeserializeOwned>(&self, spec: CommandSpec) -> Result<T, String> {
        self.run_json_timeout(spec, COMMAND_TIMEOUT).await
    }

    async fn run_json_timeout<T: DeserializeOwned>(
        &self,
        spec: CommandSpec,
        timeout: Duration,
    ) -> Result<T, String> {
        let output = self.run_output(spec, timeout).await?;
        parse_json(&output, "telex")
    }

    async fn run_output(&self, spec: CommandSpec, timeout: Duration) -> Result<String, String> {
        let child = self.spawn(spec)?;
        let result = tokio::time::timeout(timeout, child.wait_with_output()).await;
        match result {
            Ok(Ok(output)) if output.status.success() => String::from_utf8(output.stdout)
                .map_err(|error| format!("telex stdout is not UTF-8: {error}")),
            Ok(Ok(output)) => {
                let code = output.status.code().unwrap_or(1);
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(self
                    .config
                    .redact(&format!("telex exited {code}: {}", stderr.trim())))
            }
            Ok(Err(error)) => Err(format!("waiting for telex failed: {error}")),
            Err(_) => Err(format!(
                "telex command timed out after {}ms",
                timeout.as_millis()
            )),
        }
    }

    async fn run_json_lines<T: DeserializeOwned>(
        &self,
        spec: CommandSpec,
    ) -> Result<Vec<T>, String> {
        let mut child = self.spawn(spec)?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to capture telex export stdout".to_string())?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| "failed to capture telex export stderr".to_string())?;
        let stderr_task = tokio::spawn(async move {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes).await.map(|_| bytes)
        });
        let result = tokio::time::timeout(EXPORT_TIMEOUT, async {
            let mut lines = BufReader::new(stdout).lines();
            let mut parsed = Vec::new();
            while let Some(line) = lines
                .next_line()
                .await
                .map_err(|error| format!("reading telex export failed: {error}"))?
            {
                if !line.trim().is_empty() {
                    parsed.push(parse_json(&line, "export JSONL")?);
                }
            }
            let status = child
                .wait()
                .await
                .map_err(|error| format!("waiting for telex export failed: {error}"))?;
            let stderr = stderr_task
                .await
                .map_err(|error| format!("joining telex export stderr failed: {error}"))?
                .map_err(|error| format!("reading telex export stderr failed: {error}"))?;
            if !status.success() {
                return Err(self.config.redact(&format!(
                    "telex export exited {}: {}",
                    status.code().unwrap_or(1),
                    String::from_utf8_lossy(&stderr).trim()
                )));
            }
            Ok(parsed)
        })
        .await;
        result.unwrap_or_else(|_| Err("telex export timed out after 10000ms".into()))
    }

    fn spawn(&self, spec: CommandSpec) -> Result<Child, String> {
        let mut command = Command::new(&self.config.telex_executable);
        command
            .args(spec.args)
            .envs(spec.env)
            .stdin(if spec.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        configure_windows_process(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| format!("spawning telex failed: {error}"))?;
        if let Some(stdin) = spec.stdin {
            let mut pipe = child
                .stdin
                .take()
                .ok_or_else(|| "failed to open telex stdin".to_string())?;
            tokio::spawn(async move {
                let _ = pipe.write_all(&stdin).await;
                let _ = pipe.shutdown().await;
            });
        }
        Ok(child)
    }
}

impl RunningWait {
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub async fn finish(
        mut self,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<WaitExecution, String> {
        if *shutdown.borrow() {
            let _ = self.child.kill().await;
            let _ = self.child.wait().await;
            self.stdout.abort();
            self.stderr.abort();
            return Ok(WaitExecution::Shutdown);
        }
        let status = tokio::select! {
            result = self.child.wait() => {
                result.map_err(|error| format!("waiting for telex wait failed: {error}"))?
            }
            changed = shutdown.changed() => {
                if changed.is_ok() && *shutdown.borrow() {
                    let _ = self.child.kill().await;
                    let _ = self.child.wait().await;
                    self.stdout.abort();
                    self.stderr.abort();
                    return Ok(WaitExecution::Shutdown);
                }
                return Err("Station shutdown channel closed unexpectedly".into());
            }
        };
        let stdout = join_utf8(self.stdout, "wait stdout").await?;
        let stderr = join_utf8(self.stderr, "wait stderr").await?;
        Ok(WaitExecution::Exited {
            code: status.code().unwrap_or(1),
            stdout,
            stderr,
        })
    }
}

impl From<DispositionWire> for DispositionRecord {
    fn from(value: DispositionWire) -> Self {
        Self {
            id: value.id,
            message_id: value.message_id,
            recipient: value.recipient,
            state: value.state,
            note: value.note,
            by_principal: value.by_principal,
            at_ms: value.at_ms,
        }
    }
}

impl From<ReceiptWire> for SentReceipt {
    fn from(value: ReceiptWire) -> Self {
        Self {
            receipt: value.receipt,
            id: value.id,
            thread_id: value.thread_id,
            parent_id: value.parent_id,
            to: value.to,
            from: value.from,
            attention: value.attention,
            requires_disposition: value.requires_disposition,
            occupied: value.occupied,
        }
    }
}

fn export_message(
    item: ExportItemWire,
    station_address: &str,
    active_fingerprint: &str,
) -> StationMessage {
    let role = if item.message.to_addr == station_address {
        "to"
    } else if split_cc(item.message.cc.as_deref())
        .iter()
        .any(|address| address == station_address)
    {
        "cc"
    } else {
        "unknown"
    };
    let latest = latest_for(&item.dispositions, station_address);
    let required = item.message.requires_disposition && role == "to";
    let actionable = required && !latest.as_deref().is_some_and(is_terminal);
    let primary_to = item.message.to_addr.clone();
    convert_message(
        item.message,
        station_address.to_string(),
        primary_to,
        Vec::new(),
        role.to_string(),
        required,
        latest,
        actionable,
        active_fingerprint,
    )
}

fn inbox_message(item: InboxItemWire, active_fingerprint: &str) -> StationMessage {
    convert_message(
        item.message,
        item.delivered_to,
        item.primary_to,
        item.cc_recipients,
        item.delivery_role,
        item.requires_disposition_for_current_recipient,
        item.latest_disposition,
        item.actionable,
        active_fingerprint,
    )
}

#[allow(clippy::too_many_arguments)]
fn convert_message(
    wire: MessageWire,
    delivered_to: String,
    primary_to: String,
    delivery_cc: Vec<String>,
    delivery_role: String,
    requires_disposition_for_current_recipient: bool,
    latest_disposition: Option<String>,
    actionable: bool,
    active_fingerprint: &str,
) -> StationMessage {
    let cc = if delivery_cc.is_empty() {
        split_cc(wire.cc.as_deref())
    } else {
        delivery_cc
    };
    let mut message = StationMessage {
        id: wire.id,
        thread_id: wire.thread_id,
        parent_id: wire.parent_id,
        from: wire.from_addr,
        to: wire.to_addr,
        cc,
        kind: wire.kind,
        attention: wire.attention,
        requires_disposition: wire.requires_disposition,
        subject: wire.subject,
        body: wire.body,
        metadata_raw: wire.metadata,
        sent_at_ms: wire.sent_at_ms,
        created_at_ms: Some(wire.created_at_ms),
        delivered_to,
        primary_to,
        delivery_role,
        requires_disposition_for_current_recipient,
        latest_disposition,
        actionable,
        ack_pending: false,
        source_references: Vec::new(),
        metadata_error: None,
    };
    message.parse_metadata(active_fingerprint);
    message
}

fn latest_for(dispositions: &[DispositionWire], recipient: &str) -> Option<String> {
    dispositions
        .iter()
        .filter(|item| item.recipient == recipient)
        .max_by_key(|item| item.id)
        .map(|item| item.state.clone())
}

fn split_cc(cc: Option<&str>) -> Vec<String> {
    cc.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn is_terminal(value: &str) -> bool {
    matches!(value, "handled" | "rejected" | "closed")
}

fn parse_json<T: DeserializeOwned>(body: &str, label: &str) -> Result<T, String> {
    let body = body.trim();
    if body.is_empty() {
        return Err(format!("{label} response is empty"));
    }
    serde_json::from_str(body).map_err(|error| format!("invalid {label} response: {error}"))
}

async fn join_utf8(
    task: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
    label: &str,
) -> Result<String, String> {
    let bytes = task
        .await
        .map_err(|error| format!("joining telex {label} failed: {error}"))?
        .map_err(|error| format!("reading telex {label} failed: {error}"))?;
    String::from_utf8(bytes).map_err(|error| format!("telex {label} is not UTF-8: {error}"))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(windows)]
fn configure_windows_process(command: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_windows_process(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn adapter() -> TelexCli {
        let config = Arc::new(RuntimeConfig {
            station_address: "operator:rob".into(),
            ingress_address: "attention:rob".into(),
            telex_executable: "telex".into(),
            database_path: PathBuf::from(r"C:\store\operator.sqlite"),
            store_fingerprint: "sha256:active".into(),
            scope_key: "scope".into(),
        });
        TelexCli::new(config, "session-123".into())
    }

    #[test]
    fn every_child_gets_required_environment_and_explicit_selectors() {
        let cli = adapter();
        let spec = cli.spec(&["read", "--id", "7", "--full"], "operator:rob", true);
        let args: Vec<_> = spec
            .args
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            &args[..5],
            &[
                "--db",
                r"C:\store\operator.sqlite",
                "--address",
                "operator:rob",
                "--json"
            ]
        );
        assert_eq!(spec.env["TELEX_SESSION_ID"], "session-123");
        assert_eq!(spec.env["TELEX_ADDRESS"], "operator:rob");
        assert_eq!(
            spec.env["TELEX_OPERATOR_SPIKE_DB"],
            r"C:\store\operator.sqlite"
        );
    }

    #[test]
    fn wait_parser_tolerates_additive_fields_but_rejects_missing_required_fields() {
        let valid = r#"{
          "id": 9, "thread_id": 9, "parent_id": null, "from": "attention:rob",
          "to": "operator:rob", "delivered_to": "operator:rob", "primary_to": "operator:rob",
          "cc": null, "delivery_role": "to", "kind": "note", "attention": "interrupt",
          "requires_disposition": false, "requires_disposition_for_current_recipient": false,
          "subject": null, "body": "hello", "sent_at_ms": 1, "buffered_at_ms": 2,
          "lease_epoch": 3, "waiter_exit_ms": 4, "future_field": {"ok": true}
        }"#;
        assert_eq!(adapter().parse_wait_payload(valid).unwrap().id, 9);
        let missing_id = valid.replace("\"id\": 9,", "");
        assert!(adapter().parse_wait_payload(&missing_id).is_err());
        let wrong_type = valid.replace("\"thread_id\": 9", "\"thread_id\": \"9\"");
        assert!(adapter().parse_wait_payload(&wrong_type).is_err());
    }

    #[test]
    fn read_parser_double_parses_metadata_and_tolerates_additive_fields() {
        let metadata = serde_json::json!({
            "extensions": {
                "operator-station-spike": "urn:telex:experimental:operator-station-spike:v1"
            },
            "dataschema": "urn:telex:experimental:operator-station-spike:v1#escalation",
            "ext": {
                "operator-station-spike": {
                    "sourceMessages": [{
                        "id": 1,
                        "threadId": 1,
                        "storeFingerprint": "sha256:active"
                    }]
                }
            }
        })
        .to_string();
        let body = serde_json::json!({
            "message": {
                "id": 2, "thread_id": 2, "parent_id": null, "from_addr": "attention:rob",
                "to_addr": "operator:rob", "cc": null, "kind": "operator-station-spike.escalation",
                "attention": "next-checkpoint", "requires_disposition": true,
                "subject": "Decision", "body": "Choose", "metadata": metadata,
                "sent_at_ms": 1, "created_at_ms": 2, "future": 3
            },
            "dispositions": [],
            "delivery": {
                "delivered_to": "operator:rob", "primary_to": "operator:rob", "cc": [],
                "delivery_role": "to", "requires_disposition_for_current_recipient": true
            },
            "thread": [],
            "future": true
        })
        .to_string();
        let parsed: ReadResponseWire = parse_json(&body, "read").unwrap();
        let message = convert_message(
            parsed.message,
            parsed.delivery.delivered_to,
            parsed.delivery.primary_to,
            parsed.delivery.cc,
            parsed.delivery.delivery_role,
            parsed.delivery.requires_disposition_for_current_recipient,
            None,
            true,
            "sha256:active",
        );
        assert_eq!(message.source_references.len(), 1);
        assert!(message.metadata_raw.unwrap().starts_with('{'));
    }

    #[test]
    fn lifecycle_fixtures_cover_every_strict_response_shape() {
        let attach: AttachWire = parse_json(
            include_str!("../../fixtures/telex-cli/attach.json"),
            "attach",
        )
        .unwrap();
        assert_eq!(attach.address, "operator:rob");
        assert_eq!(attach.lease_epoch, 1);

        let status: StatusWire = parse_json(
            include_str!("../../fixtures/telex-cli/status.json"),
            "status",
        )
        .unwrap();
        assert!(status.occupancy.occupied);
        assert_eq!(status.live_waiters_count, Some(1));

        let station: StationStatusWire = parse_json(
            include_str!("../../fixtures/telex-cli/station-status.json"),
            "station status",
        )
        .unwrap();
        assert_eq!(station.count, station.stations.len());
        assert_eq!(station.stations[0].station_health, "armed");

        let stopped: StopWire = parse_json(
            include_str!("../../fixtures/telex-cli/station-stop.json"),
            "station stop",
        )
        .unwrap();
        assert!(stopped.detached);
        assert_eq!(stopped.waiters_after, 0);
    }
}
