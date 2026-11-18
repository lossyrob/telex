//! The Station engine: attends configured Telex addresses through the supported
//! Application Client, keeps a durable local projection (ingest before ack), and
//! turns human actions into ordinary Telex replies and exact-recipient
//! dispositions.

use crate::card;
use crate::model::*;
use crate::org;
use crate::policy;
use crate::view;
use anyhow::{anyhow, bail, Context as _, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use crate::port::{Port, PortError, Reconciled, Sent};
use crate::port_cli::CliPort;
use crate::port_client::ClientPort;
use telex::application_client::ApplicationDaemonBootstrap;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

#[derive(Clone, Debug)]
pub enum TelexMode {
    /// Isolated Telex root with a pinned executable (prototype / tests).
    Sandbox { root: PathBuf, executable: PathBuf },
    /// The user's installed Telex.
    Installed { trusted_root: PathBuf, cli: Option<PathBuf> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortKind {
    /// Supported `telex::application_client` (production seam).
    ApplicationClient,
    /// Drive the `telex` CLI as an ordinary session (macOS prototype seam).
    Cli,
}

impl PortKind {
    /// macOS cannot use the Application Client bootstrap today; see `port.rs`.
    pub fn platform_default() -> PortKind {
        if cfg!(target_os = "macos") {
            PortKind::Cli
        } else {
            PortKind::ApplicationClient
        }
    }
}

#[derive(Clone, Debug)]
pub struct StationConfig {
    pub data_dir: PathBuf,
    pub telex: TelexMode,
    pub port: PortKind,
    pub backend: Option<String>,
    pub operator_name: String,
    pub default_address: String,
}

impl StationConfig {
    fn cli(&self) -> Option<PathBuf> {
        match &self.telex {
            TelexMode::Sandbox { executable, .. } => Some(executable.clone()),
            TelexMode::Installed { cli, .. } => cli.clone(),
        }
    }

    /// Telex selection is process-wide, so the sandbox is applied once at startup.
    pub fn apply_env(&self) -> Result<()> {
        if let TelexMode::Sandbox { root, executable } = &self.telex {
            std::fs::create_dir_all(root)?;
            std::env::set_var("TELEX_HOME", root.join("home"));
            std::env::set_var("TELEX_RUN_DIR", root.join("run"));
            std::env::set_var("TELEX_DB", root.join("telex.db"));
            std::env::set_var("TELEX_CONFIG", root.join("config.toml"));
            std::env::set_var("TELEX_INSTALL_ROOT", root.join("install"));
            std::env::set_var("XDG_STATE_HOME", root.join("state"));
            for key in ["TELEX_BACKEND", "TELEX_ADDRESS", "TELEX_SESSION_ID", "TELEX_SESSION_PID"] {
                std::env::remove_var(key);
            }
            if !root.join("config.toml").exists() {
                let out = std::process::Command::new(executable)
                    .args(["init", "--json"])
                    .output()
                    .with_context(|| format!("running {} init", executable.display()))?;
                if !out.status.success() {
                    bail!("telex init failed: {}", String::from_utf8_lossy(&out.stderr));
                }
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct Runtime {
    connected: bool,
    connect_error: Option<String>,
    store_id: String,
    last_error: BTreeMap<String, String>,
    last_receive_ms: BTreeMap<String, i64>,
    attached: BTreeSet<String>,
    port_kind: String,
    port_note: Option<String>,
}

#[derive(Default)]
struct Coalesce {
    window_end_ms: i64,
    held: Vec<(String, Tier)>,
}

pub struct Engine {
    cfg: StationConfig,
    store: Mutex<Store>,
    state_path: PathBuf,
    client: Mutex<Option<Port>>,
    events: broadcast::Sender<StationEvent>,
    rev: AtomicU64,
    presence: Mutex<BTreeMap<String, Presence>>,
    health: Mutex<BTreeMap<String, HealthView>>,
    runtime: Mutex<Runtime>,
    tasks: Mutex<BTreeMap<String, JoinHandle<()>>>,
    background: Mutex<Vec<JoinHandle<()>>>,
    coalesce: Mutex<Coalesce>,
    telex_build: Option<String>,
}

impl Engine {
    pub fn subscribe(&self) -> broadcast::Receiver<StationEvent> {
        self.events.subscribe()
    }

    pub async fn start(cfg: StationConfig) -> Result<Arc<Engine>> {
        cfg.apply_env()?;
        std::fs::create_dir_all(&cfg.data_dir)?;
        let state_path = cfg.data_dir.join("station-state.json");
        let mut store: Store = match std::fs::read(&state_path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing {}", state_path.display()))?,
            Err(_) => Store::default(),
        };
        if store.version == 0 {
            store.version = 1;
            store.settings.operator_name = cfg.operator_name.clone();
            store.settings.addresses.push(AttendedAddress {
                address: cfg.default_address.clone(),
                label: Some("Primary".into()),
                enabled: true,
            });
        }
        // Authored messages that never reached a decision are surfaced, never resent blindly.
        for item in store.outbox.iter_mut().filter(|o| o.state == "sending") {
            item.state = "indeterminate".into();
            item.error = Some("Station stopped before Telex confirmed this message".into());
        }
        let telex_build = cfg.cli().and_then(|cli| {
            std::process::Command::new(cli).arg("--version").output().ok().map(|o| {
                String::from_utf8_lossy(&o.stdout).trim().to_string()
            })
        });
        let (events, _) = broadcast::channel(512);
        let engine = Arc::new(Engine {
            cfg,
            store: Mutex::new(store),
            state_path,
            client: Mutex::new(None),
            events,
            rev: AtomicU64::new(1),
            presence: Mutex::new(BTreeMap::new()),
            health: Mutex::new(BTreeMap::new()),
            runtime: Mutex::new(Runtime::default()),
            tasks: Mutex::new(BTreeMap::new()),
            background: Mutex::new(Vec::new()),
            coalesce: Mutex::new(Coalesce::default()),
            telex_build,
        });
        engine.save()?;
        let supervisor = tokio::spawn(engine.clone().supervise());
        let ticker = tokio::spawn(engine.clone().tick());
        engine.background.lock().unwrap().extend([supervisor, ticker]);
        Ok(engine)
    }

    pub fn shutdown(&self) {
        for (_, task) in std::mem::take(&mut *self.tasks.lock().unwrap()) {
            task.abort();
        }
        for task in std::mem::take(&mut *self.background.lock().unwrap()) {
            task.abort();
        }
    }

    // ------------------------------------------------------------ persistence

    fn save_locked(&self, store: &Store) -> Result<()> {
        let tmp = self.state_path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(store)?)?;
        std::fs::rename(&tmp, &self.state_path)?;
        Ok(())
    }

    fn save(&self) -> Result<()> {
        let store = self.store.lock().unwrap();
        self.save_locked(&store)
    }

    /// Apply a mutation, persist it, and tell the UI. The mutation is durable
    /// before this returns, which is what makes ack-after-ingest honest.
    fn mutate<R>(&self, f: impl FnOnce(&mut Store) -> R) -> Result<R> {
        let result = {
            let mut store = self.store.lock().unwrap();
            let result = f(&mut store);
            self.save_locked(&store)?;
            result
        };
        self.changed();
        Ok(result)
    }

    fn changed(&self) {
        let rev = self.rev.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.events.send(StationEvent::Changed { rev });
    }

    fn client(&self) -> Result<Port> {
        self.client.lock().unwrap().clone().ok_or_else(|| anyhow!("Station is not connected to Telex yet"))
    }

    pub fn snapshot(&self) -> Snapshot {
        let store = self.store.lock().unwrap();
        let presence = self.presence.lock().unwrap();
        let runtime = self.runtime.lock().unwrap();
        let mut health: Vec<HealthView> = Vec::new();
        let known = self.health.lock().unwrap();
        for a in store.settings.addresses.iter().filter(|a| a.enabled) {
            let mut h = known.get(&a.address).cloned().unwrap_or_else(|| HealthView {
                address: a.address.clone(),
                principal_verification: "unavailable".into(),
                ..Default::default()
            });
            h.last_error = runtime.last_error.get(&a.address).cloned();
            h.last_receive_ms = runtime.last_receive_ms.get(&a.address).copied();
            health.push(h);
        }
        view::build(
            &store,
            view::Extras {
                rev: self.rev.load(Ordering::SeqCst),
                station: StationInfo {
                    operator_name: store.settings.operator_name.clone(),
                    store_id: runtime.store_id.clone(),
                    backend: self.cfg.backend.clone().unwrap_or_else(|| "default".into()),
                    connected: runtime.connected,
                    connect_error: runtime.connect_error.clone(),
                    telex_build: self.telex_build.clone(),
                    sandbox: matches!(self.cfg.telex, TelexMode::Sandbox { .. }),
                    port: runtime.port_kind.clone(),
                    port_note: runtime.port_note.clone(),
                },
                presence: &presence,
                health,
            },
        )
    }

    // ---------------------------------------------------------- telex runtime

    async fn connect(&self) -> Result<Port, PortError> {
        match self.cfg.port {
            PortKind::Cli => {
                let exe = self
                    .cfg
                    .cli()
                    .ok_or_else(|| PortError::new("the CLI port needs a telex executable (STATION_TELEX_EXE)"))?;
                // Any CLI call proves the binary runs before we report "connected".
                let port = CliPort::new(exe, &self.cfg.operator_name, self.cfg.backend.clone());
                Ok(Arc::new(port))
            }
            PortKind::ApplicationClient => {
                let bootstrap = match &self.cfg.telex {
                    TelexMode::Sandbox { executable, .. } => {
                        ApplicationDaemonBootstrap::ExactExecutable { executable: executable.clone() }
                    }
                    TelexMode::Installed { trusted_root, .. } => {
                        ApplicationDaemonBootstrap::InstalledCurrent { trusted_root: trusted_root.clone() }
                    }
                };
                let port = ClientPort::connect(
                    format!("telex-station-prototype:{}", self.cfg.operator_name),
                    self.cfg.backend.clone(),
                    bootstrap,
                    self.cfg.cli(),
                )
                .await?;
                Ok(Arc::new(port))
            }
        }
    }

    async fn supervise(self: Arc<Self>) {
        let client = loop {
            match self.connect().await {
                Ok(client) => break client,
                Err(error) => {
                    {
                        let mut rt = self.runtime.lock().unwrap();
                        rt.connected = false;
                        rt.connect_error = Some(error.to_string());
                    }
                    self.changed();
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
            }
        };
        {
            let mut rt = self.runtime.lock().unwrap();
            rt.connected = true;
            rt.connect_error = None;
            rt.store_id = client.store_id();
            rt.port_kind = client.kind().to_string();
            rt.port_note = client.note();
        }
        *self.client.lock().unwrap() = Some(client.clone());
        self.changed();
        let addresses: Vec<String> = {
            let store = self.store.lock().unwrap();
            store.settings.addresses.iter().filter(|a| a.enabled).map(|a| a.address.clone()).collect()
        };
        for address in addresses {
            self.spawn_receiver(client.clone(), address);
        }
    }

    fn spawn_receiver(self: &Arc<Self>, client: Port, address: String) {
        let task = tokio::spawn(self.clone().receive_loop(client, address.clone()));
        if let Some(previous) = self.tasks.lock().unwrap().insert(address, task) {
            previous.abort();
        }
    }

    fn set_address_error(&self, address: &str, error: Option<String>) {
        let changed = {
            let mut rt = self.runtime.lock().unwrap();
            let before = rt.last_error.get(address).cloned();
            match &error {
                Some(e) => rt.last_error.insert(address.to_string(), e.clone()),
                None => rt.last_error.remove(address),
            };
            before != error
        };
        if changed {
            self.changed();
        }
    }

    async fn receive_loop(self: Arc<Self>, client: Port, address: String) {
        let description = format!("Telex Station (human-attended: {})", self.cfg.operator_name);
        let mut attached = false;
        let mut first_sync = true;
        loop {
            if !attached {
                match client.attach(&address, &description).await {
                    Ok(()) => {
                        attached = true;
                        self.runtime.lock().unwrap().attached.insert(address.clone());
                        self.set_address_error(&address, None);
                        self.refresh_health().await;
                    }
                    Err(error) => {
                        self.runtime.lock().unwrap().attached.remove(&address);
                        self.set_address_error(&address, Some(format!("attach: {error}")));
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        continue;
                    }
                }
            }
            if let Err(error) = self.sync_history(&client, &address, first_sync).await {
                self.set_address_error(&address, Some(format!("history: {error:#}")));
            }
            first_sync = false;
            match client.receive(&address, 10_000).await {
                Ok(Some(message)) => {
                    self.runtime.lock().unwrap().last_receive_ms.insert(address.clone(), now_ms());
                    if let Err(error) = self.ingest_delivery(&client, message).await {
                        self.set_address_error(&address, Some(format!("ingest: {error:#}")));
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    } else {
                        self.set_address_error(&address, None);
                    }
                }
                Ok(None) => self.set_address_error(&address, None),
                Err(error) => {
                    self.set_address_error(&address, Some(format!("receive: {error}")));
                    attached = false;
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    }

    /// Exact-delivery receive path: persist first, then acknowledge.
    async fn ingest_delivery(&self, client: &Port, message: StoredMessage) -> Result<()> {
        let id = message.id;
        self.ingest(message.clone(), false)?;
        client.acknowledge(&message).await.map_err(|e| anyhow!("acknowledge: {e}"))?;
        self.mutate(|store| {
            if let Some(m) = store.messages.get_mut(&id) {
                m.acked = true;
            }
        })?;
        Ok(())
    }

    /// Bounded history sync: recovers deliveries consumed elsewhere or missed
    /// while Station was down, picks up CC rows, and refreshes dispositions.
    async fn sync_history(&self, client: &Port, address: &str, startup: bool) -> Result<()> {
        let cursor = if startup {
            0
        } else {
            self.store.lock().unwrap().history_cursor.get(address).copied().unwrap_or(0)
        };
        let (rows, cursor) = client.history(address, cursor).await.map_err(|e| anyhow!("{e}"))?;
        let mut recovered: Vec<NotificationRecord> = Vec::new();
        for message in rows {
            if let Some(record) = self.ingest(message, startup)? {
                recovered.push(record);
            }
        }
        let moved = self.store.lock().unwrap().history_cursor.get(address).copied() != Some(cursor);
        if moved {
            self.mutate(|store| {
                store.history_cursor.insert(address.to_string(), cursor);
            })?;
        }
        if startup && !recovered.is_empty() {
            // One summary instead of a burst of startup banners.
            let top = recovered.iter().map(|r| r.effective_tier).max().unwrap_or(Tier::Routine);
            let blocked = recovered.iter().filter(|r| r.effective_tier == Tier::Blocked).count();
            let settings = self.store.lock().unwrap().settings.clone();
            let title = format!("{} waiting from while Station was closed", recovered.len());
            let body = if blocked > 0 {
                format!("{blocked} blocked · {} other", recovered.len() - blocked)
            } else {
                "Nothing blocked".to_string()
            };
            let ctx = policy::Context { tier: Some(top), ..Default::default() };
            let record = policy::decide(&settings, &ctx, "summary", None, None, title, body);
            self.publish_notification(record)?;
        }
        Ok(())
    }

    /// Insert or merge one inbound row. Returns a would-be notification when the
    /// row is new and `defer_notify` asked the caller to summarize instead.
    fn ingest(&self, message: StoredMessage, defer_notify: bool) -> Result<Option<NotificationRecord>> {
        let id = message.id;
        // Nothing to do for a row we already hold unchanged: avoid a disk write per poll.
        {
            let store = self.store.lock().unwrap();
            if let Some(existing) = store.messages.get(&id) {
                let same = (message.delivery.is_none() || existing.delivery == message.delivery)
                    && (message.disposition.is_none() || existing.disposition == message.disposition)
                    && (!message.acked || existing.acked)
                    && (message.direction == Direction::Inbound
                        || message.receipt.as_ref().map_or(true, |r| {
                            existing.receipt.as_ref().map_or(false, |e| {
                                e.disposition == r.disposition && (e.consumed || !r.consumed)
                            })
                        }));
                if same {
                    return Ok(None);
                }
            }
        }
        let is_new = self.mutate(|store| {
            if let Some(existing) = store.messages.get_mut(&id) {
                if existing.direction == Direction::Inbound {
                    if message.delivery.is_some() {
                        existing.delivery = message.delivery.clone();
                    }
                    if message.disposition.is_some() {
                        existing.disposition = message.disposition.clone();
                    }
                    existing.acked |= message.acked;
                } else if let Some(fresh) = &message.receipt {
                    let receipt = existing.receipt.get_or_insert_with(Receipt::default);
                    receipt.consumed |= fresh.consumed;
                    if fresh.disposition.is_some() {
                        receipt.disposition = fresh.disposition.clone();
                    }
                }
                return false;
            }
            if message.direction == Direction::Outbound {
                // Authored elsewhere on this address (or before local state existed).
                let to = message.to.clone();
                store.agents.entry(to.clone()).or_insert_with(|| AgentRecord {
                    address: to,
                    first_seen_ms: message.sent_at_ms,
                    last_seen_ms: message.sent_at_ms,
                    ..Default::default()
                });
                store.threads.entry(message.thread_id).or_default();
                store.messages.insert(id, message.clone());
                return false;
            }
            if let Some(from) = &message.from {
                let agent = store.agents.entry(from.clone()).or_insert_with(|| AgentRecord {
                    address: from.clone(),
                    first_seen_ms: message.sent_at_ms,
                    ..Default::default()
                });
                agent.last_seen_ms = agent.last_seen_ms.max(message.sent_at_ms);
                if let Some(card) = card::parse(message.metadata.as_deref()).card {
                    agent.card.merge_from(&card);
                }
            }
            let local = store.threads.entry(message.thread_id).or_default();
            if message.role == "primary" && message.tier() != Tier::Fyi {
                // New work always wakes a snoozed thread.
                local.snooze_until_ms = None;
            }
            store.messages.insert(id, message.clone());
            true
        })?;
        if !is_new {
            return Ok(None);
        }
        // Rows that arrive already resolved (history recovery) never notify.
        let unresolved = !message.is_terminal_disposition();
        if !unresolved {
            return Ok(None);
        }
        let record = self.decide_for_message(&message, "new");
        if defer_notify {
            let counts = record.badge || record.banner;
            self.mutate(|store| {
                let mut quiet = record.clone();
                quiet.banner = false;
                quiet.sound = false;
                quiet.demand = false;
                quiet.outcome = "quiet".into();
                quiet.reason = "arrived while Station was closed; summarized".into();
                push_notification(store, quiet);
            })?;
            return Ok(counts.then_some(record));
        }
        self.publish_notification(record)?;
        Ok(None)
    }

    fn decide_for_message(&self, message: &StoredMessage, kind: &str) -> NotificationRecord {
        let store = self.store.lock().unwrap();
        let from = message.from.clone().unwrap_or_default();
        let agent = store.agents.get(&from);
        let local = store.threads.get(&message.thread_id);
        let project_id = local
            .and_then(|l| l.project_id.clone())
            .filter(|id| org::project_exists(&store.projects, id))
            .or_else(|| agent.and_then(|a| org::resolve_agent(&store, a).0));
        let project_notify = project_id.and_then(|id| {
            org::lineage(&store.projects, &id)
                .into_iter()
                .rev()
                .find(|p| p.notify != NotifyOverride::Inherit)
                .map(|p| (p.notify, p.name))
        });
        let ctx = policy::Context {
            tier: Some(message.tier()),
            is_cc: message.role != "primary",
            thread_muted: local.map_or(false, |l| l.muted),
            agent_notify: agent.map(|a| a.notify).unwrap_or_default(),
            project_notify,
            ..Default::default()
        };
        let name = agent
            .map(|a| {
                a.nickname.clone().or_else(|| a.card.name.clone()).unwrap_or_else(|| view::short_address(&a.address))
            })
            .unwrap_or_else(|| view::short_address(&from));
        let place = agent
            .and_then(|a| match (&a.card.harness, &a.card.host) {
                (Some(h), Some(host)) => Some(format!(" ({h} @ {host})")),
                (None, Some(host)) => Some(format!(" (@ {host})")),
                (Some(h), None) => Some(format!(" ({h})")),
                _ => None,
            })
            .unwrap_or_default();
        let prefix = if kind == "reminder" { "Still waiting · " } else { "" };
        let title = format!("{prefix}{} · {name}{place}", message.tier().label());
        let first_line: String = message.body.lines().next().unwrap_or("").chars().take(160).collect();
        let body = match &message.subject {
            Some(subject) if !subject.trim().is_empty() => format!("{subject}\n{first_line}"),
            _ => first_line,
        };
        policy::decide(&store.settings, &ctx, kind, Some(message.thread_id), Some(message.id), title, body)
    }

    /// Record the decision as evidence and, when it is a banner, hand it to the shell.
    fn publish_notification(self: &Self, mut record: NotificationRecord) -> Result<()> {
        let window_s = self.store.lock().unwrap().settings.coalesce_window_s as i64;
        let mut start_window = false;
        if record.banner && !record.demand && record.kind == "new" && window_s > 0 {
            let mut c = self.coalesce.lock().unwrap();
            if record.at_ms < c.window_end_ms {
                c.held.push((record.title.clone(), record.effective_tier));
                record.banner = false;
                record.sound = false;
                record.outcome = "quiet".into();
                record.reason = "coalesced into a summary banner".into();
            } else {
                c.window_end_ms = record.at_ms + window_s * 1000;
                c.held.clear();
                start_window = true;
            }
        }
        self.mutate(|store| push_notification(store, record.clone()))?;
        if record.banner {
            let _ = self.events.send(StationEvent::Notify { record });
        }
        if start_window {
            // The flush is driven by `tick`, which checks the window end.
        }
        Ok(())
    }

    fn flush_coalesced(&self) {
        let held = {
            let mut c = self.coalesce.lock().unwrap();
            if c.held.is_empty() || now_ms() < c.window_end_ms {
                return;
            }
            std::mem::take(&mut c.held)
        };
        let settings = self.store.lock().unwrap().settings.clone();
        let top = held.iter().map(|(_, t)| *t).max().unwrap_or(Tier::Routine);
        let ctx = policy::Context { tier: Some(top), ..Default::default() };
        let names: BTreeSet<&str> = held.iter().map(|(t, _)| t.as_str()).collect();
        let mut record = policy::decide(
            &settings,
            &ctx,
            "summary",
            None,
            None,
            if held.len() == 1 {
                "1 more message needs you".to_string()
            } else {
                format!("{} more messages need you", held.len())
            },
            names.into_iter().take(4).collect::<Vec<_>>().join("\n"),
        );
        record.sound = false;
        record.demand = false;
        if self.mutate(|store| push_notification(store, record.clone())).is_ok() && record.banner {
            let _ = self.events.send(StationEvent::Notify { record });
        }
    }

    // ------------------------------------------------------- periodic upkeep

    async fn tick(self: Arc<Self>) {
        let mut n: u64 = 0;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            n += 1;
            self.flush_coalesced();
            if n % 3 == 0 {
                self.refresh_presence().await;
                self.refresh_receipts().await;
            }
            if n % 5 == 0 {
                self.refresh_health().await;
            }
            if n % 15 == 0 {
                self.send_reminders();
                // Snoozes and focus windows expire on the clock, not on an event.
                self.changed();
            }
        }
    }

    async fn refresh_presence(&self) {
        let Ok(client) = self.client() else { return };
        let Ok(next) = client.presence().await else { return };
        let changed = {
            let mut presence = self.presence.lock().unwrap();
            let differs = presence.len() != next.len()
                || next.iter().any(|(k, v)| {
                    presence.get(k).map_or(true, |p| p.state != v.state || p.detail != v.detail)
                });
            *presence = next;
            differs
        };
        if changed {
            self.changed();
        }
    }

    async fn refresh_receipts(&self) {
        let Ok(client) = self.client() else { return };
        let now = now_ms();
        let pending: Vec<StoredMessage> = {
            let store = self.store.lock().unwrap();
            store
                .messages
                .values()
                .filter(|m| m.direction == Direction::Outbound && now - m.sent_at_ms < 48 * 3_600_000)
                .filter(|m| {
                    m.receipt.as_ref().map_or(true, |r| {
                        !r.consumed || !matches!(r.disposition.as_deref(), Some("handled" | "rejected" | "closed"))
                    })
                })
                .cloned()
                .collect()
        };
        for message in pending {
            let Ok(fresh) = client.receipt(&message).await else { continue };
            let differs = message
                .receipt
                .as_ref()
                .map_or(true, |r| r.consumed != fresh.consumed || r.disposition != fresh.disposition);
            if differs {
                let id = message.id;
                let _ = self.mutate(|store| {
                    if let Some(m) = store.messages.get_mut(&id) {
                        m.receipt = Some(fresh);
                    }
                });
            }
        }
    }

    async fn refresh_health(&self) {
        let Ok(client) = self.client() else { return };
        let Ok(records) = client.health().await else { return };
        let next: BTreeMap<String, HealthView> = records.into_iter().map(|h| (h.address.clone(), h)).collect();
        let changed = {
            let mut health = self.health.lock().unwrap();
            let differs = health.len() != next.len()
                || next.iter().any(|(k, v)| {
                    health.get(k).map_or(true, |h| {
                        h.registered != v.registered
                            || h.receive_ready != v.receive_ready
                            || h.degraded != v.degraded
                            || h.pending_unconsumed != v.pending_unconsumed
                    })
                });
            *health = next;
            differs
        };
        if changed {
            self.changed();
        }
    }

    fn send_reminders(&self) {
        let now = now_ms();
        let due: Vec<StoredMessage> = {
            let store = self.store.lock().unwrap();
            let mut by_thread: BTreeMap<i64, Vec<&StoredMessage>> = BTreeMap::new();
            for m in store.messages.values() {
                by_thread.entry(m.thread_id).or_default().push(m);
            }
            let default_local = ThreadLocal::default();
            by_thread
                .iter()
                .filter_map(|(thread_id, messages)| {
                    let local = store.threads.get(thread_id).unwrap_or(&default_local);
                    let state = view::thread_state(messages, local, false, now);
                    if state.status != "needs-me" || state.pending.is_empty() {
                        return None;
                    }
                    let after = store.settings.policy(state.tier).remind_after_min as i64 * 60_000;
                    if after == 0 {
                        return None;
                    }
                    let newest = messages.iter().filter(|m| state.pending.contains(&m.id)).max_by_key(|m| m.id)?;
                    let last = store.reminded_at_ms.get(thread_id).copied().unwrap_or(newest.ingested_at_ms);
                    (now - last >= after).then(|| (*newest).clone())
                })
                .collect()
        };
        for message in due {
            let record = self.decide_for_message(&message, "reminder");
            let _ = self.mutate(|store| {
                store.reminded_at_ms.insert(message.thread_id, now);
            });
            let _ = self.publish_notification(record);
        }
    }

    // ------------------------------------------------------- human authoring

    fn thread_messages(store: &Store, thread_id: i64) -> Vec<StoredMessage> {
        store.messages.values().filter(|m| m.thread_id == thread_id).cloned().collect()
    }

    /// Returns false when the active port cannot record that state.
    async fn record_disposition(
        &self,
        client: &Port,
        message: &StoredMessage,
        state: &str,
        note: Option<&str>,
    ) -> Result<bool> {
        let recorded = client.disposition(message, state, note).await.map_err(|e| anyhow!("{state}: {e}"))?;
        if recorded {
            let id = message.id;
            let state = state.to_string();
            self.mutate(|store| {
                if let Some(m) = store.messages.get_mut(&id) {
                    m.disposition = Some(state);
                    m.acked = true;
                }
            })?;
        }
        Ok(recorded)
    }

    /// Handle every open obligation in the thread, then mark it resolved locally.
    async fn handle_thread(&self, thread_id: i64, note: &str) -> Result<Vec<String>> {
        let client = self.client()?;
        let messages = {
            let store = self.store.lock().unwrap();
            Self::thread_messages(&store, thread_id)
        };
        let mut errors = Vec::new();
        for m in messages.iter().filter(|m| m.open_obligation()) {
            if let Err(error) = self.record_disposition(&client, m, "handled", Some(note)).await {
                errors.push(format!("#{}: {error:#}", m.id));
            }
        }
        let top = messages.iter().map(|m| m.id).max().unwrap_or(0);
        self.mutate(|store| {
            let local = store.threads.entry(thread_id).or_default();
            local.done_through = local.done_through.max(top);
            local.read_through = local.read_through.max(top);
            local.snooze_until_ms = None;
            local.pinned = false;
        })?;
        Ok(errors)
    }

    fn adopt_send(&self, item: &OutboxItem, result: &Sent) -> Result<()> {
        let local_id = item.local_id.clone();
        let message = StoredMessage {
            id: result.message_id,
            thread_id: result.thread_id,
            parent_id: item.reply_to,
            direction: Direction::Outbound,
            from: Some(result.sender.clone()),
            to: result.recipient.clone(),
            cc: Vec::new(),
            kind: if item.reply_to.is_some() { "reply".into() } else { "note".into() },
            attention: item.attention.clone(),
            requires_disposition: item.requires_disposition,
            subject: item.subject.clone(),
            body: item.body.clone(),
            metadata: item.metadata.clone(),
            sent_at_ms: now_ms(),
            ingested_at_ms: now_ms(),
            role: "author".into(),
            station_address: result.sender.clone(),
            delivery: None,
            acked: false,
            disposition: None,
            receipt: Some(Receipt {
                occupied_at_send: result.occupied,
                consumed: false,
                disposition: None,
                refreshed_at_ms: now_ms(),
            }),
            recovery: item.recovery.clone(),
            source: "authored".into(),
        };
        self.mutate(|store| {
            store.outbox.retain(|o| o.local_id != local_id);
            let top_inbound = store
                .messages
                .values()
                .filter(|m| m.thread_id == message.thread_id && m.direction == Direction::Inbound)
                .map(|m| m.id)
                .max()
                .unwrap_or(0);
            let local = store.threads.entry(message.thread_id).or_default();
            local.read_through = local.read_through.max(top_inbound);
            local.snooze_until_ms = None;
            local.pinned = false;
            let to = message.to.clone();
            store.agents.entry(to.clone()).or_insert_with(|| AgentRecord {
                address: to,
                first_seen_ms: now_ms(),
                last_seen_ms: now_ms(),
                ..Default::default()
            });
            store.messages.insert(message.id, message);
        })
    }

    fn fail_outbox(&self, local_id: &str, error: &PortError) -> Result<()> {
        let (uncertain, text) = (error.uncertain, error.message.clone());
        self.mutate(|store| {
            if let Some(item) = store.outbox.iter_mut().find(|o| o.local_id == local_id) {
                item.state = if uncertain { "indeterminate".into() } else { "failed".into() };
                item.error = Some(text);
            }
        })
    }

    fn outbox_item(&self, local_id: &str) -> Result<OutboxItem> {
        let store = self.store.lock().unwrap();
        store.outbox.iter().find(|o| o.local_id == local_id).cloned().ok_or_else(|| anyhow!("no such outbox item"))
    }

    async fn dispatch_outbox(&self, local_id: &str) -> Result<Value> {
        let client = self.client()?;
        let item = self.outbox_item(local_id)?;
        // Persist recovery evidence before the first attempt.
        let recovery = match client.prepare(&item).await {
            Ok(recovery) => recovery,
            Err(error) => {
                self.fail_outbox(local_id, &error)?;
                return Err(anyhow!("{error}"));
            }
        };
        self.mutate(|store| {
            if let Some(item) = store.outbox.iter_mut().find(|o| o.local_id == local_id) {
                if recovery.is_some() {
                    item.recovery = recovery;
                }
                item.state = "sending".into();
                item.error = None;
            }
        })?;
        let item = self.outbox_item(local_id)?;
        match client.submit(&item).await {
            Ok(result) => {
                self.adopt_send(&item, &result)?;
                let mut handle_errors = Vec::new();
                if item.handle_on_accept {
                    // Ordered "Reply & Handle": the terminal disposition only after
                    // Telex durably accepted the reply.
                    handle_errors = self
                        .handle_thread(result.thread_id, &format!("answered in reply #{}", result.message_id))
                        .await?;
                }
                Ok(json!({
                    "message_id": result.message_id,
                    "thread_id": result.thread_id,
                    "occupied": result.occupied,
                    "replayed": result.replayed,
                    "handle_errors": handle_errors,
                }))
            }
            Err(error) => {
                self.fail_outbox(local_id, &error)?;
                Err(anyhow!("{error}"))
            }
        }
    }

    pub async fn reply(
        &self,
        thread_id: i64,
        body: String,
        attention: Option<String>,
        handle: bool,
        reply_to: Option<i64>,
    ) -> Result<Value> {
        if body.trim().is_empty() {
            bail!("reply is empty");
        }
        let (target, operator) = {
            let store = self.store.lock().unwrap();
            let messages = Self::thread_messages(&store, thread_id);
            let target = match reply_to {
                Some(id) => messages.iter().find(|m| m.id == id).cloned(),
                None => messages
                    .iter()
                    .filter(|m| m.direction == Direction::Inbound)
                    .max_by_key(|m| (m.role == "primary", m.id))
                    .cloned(),
            };
            (target, store.settings.operator_name.clone())
        };
        let Some(target) = target else {
            bail!("this thread has no message from the agent to reply to yet");
        };
        let attention = attention.unwrap_or_else(|| {
            if target.tier() == Tier::Blocked { "interrupt".into() } else { "next-checkpoint".into() }
        });
        let item = OutboxItem {
            local_id: format!("o-{}", uuid::Uuid::new_v4().simple()),
            operation_id: format!("station-reply-{}", uuid::Uuid::new_v4().simple()),
            thread_id: Some(thread_id),
            reply_to: Some(target.id),
            sender: target.station_address.clone(),
            to: target.from.clone().unwrap_or_default(),
            subject: None,
            body,
            attention,
            requires_disposition: false,
            metadata: Some(card::human_metadata(&operator)),
            handle_on_accept: handle,
            created_at_ms: now_ms(),
            state: "sending".into(),
            error: None,
            recovery: None,
        };
        let local_id = item.local_id.clone();
        self.mutate(|store| store.outbox.push(item))?;
        self.dispatch_outbox(&local_id).await
    }

    pub async fn compose(
        &self,
        to: String,
        subject: Option<String>,
        body: String,
        attention: String,
        requires_disposition: bool,
        from: Option<String>,
    ) -> Result<Value> {
        if to.trim().is_empty() || body.trim().is_empty() {
            bail!("a recipient and a message are required");
        }
        let (sender, operator) = {
            let store = self.store.lock().unwrap();
            let sender = from
                .or_else(|| store.settings.addresses.iter().find(|a| a.enabled).map(|a| a.address.clone()))
                .ok_or_else(|| anyhow!("no attended address to send from"))?;
            (sender, store.settings.operator_name.clone())
        };
        let item = OutboxItem {
            local_id: format!("o-{}", uuid::Uuid::new_v4().simple()),
            operation_id: format!("station-send-{}", uuid::Uuid::new_v4().simple()),
            thread_id: None,
            reply_to: None,
            sender,
            to: to.trim().to_string(),
            subject: subject.filter(|s| !s.trim().is_empty()),
            body,
            attention,
            requires_disposition,
            metadata: Some(card::human_metadata(&operator)),
            handle_on_accept: false,
            created_at_ms: now_ms(),
            state: "sending".into(),
            error: None,
            recovery: None,
        };
        let local_id = item.local_id.clone();
        self.mutate(|store| store.outbox.push(item))?;
        self.dispatch_outbox(&local_id).await
    }

    /// Reconcile an uncertain authored message instead of resending blindly.
    async fn check_outbox(&self, local_id: &str) -> Result<Value> {
        let client = self.client()?;
        let item = self.outbox_item(local_id)?;
        match client.reconcile(&item).await.map_err(|e| anyhow!("{e}"))? {
            Reconciled::Accepted(result) => {
                self.adopt_send(&item, &result)?;
                Ok(json!({ "outcome": "accepted", "message_id": result.message_id }))
            }
            Reconciled::NotRecorded => {
                self.mutate(|store| {
                    if let Some(o) = store.outbox.iter_mut().find(|o| o.local_id == local_id) {
                        o.state = "failed".into();
                        o.error = Some("Telex confirms this was not recorded. Safe to retry or discard.".into());
                    }
                })?;
                Ok(json!({ "outcome": "not-recorded" }))
            }
            Reconciled::Unknown(detail) => {
                self.mutate(|store| {
                    if let Some(o) = store.outbox.iter_mut().find(|o| o.local_id == local_id) {
                        o.error = Some(detail.clone());
                    }
                })?;
                Ok(json!({ "outcome": "unknown", "detail": detail }))
            }
        }
    }

    // --------------------------------------------------------- command surface

    pub async fn call(self: &Arc<Self>, cmd: &str, args: Value) -> Result<Value> {
        let s = |key: &str| args.get(key).and_then(Value::as_str).map(str::to_string);
        let i = |key: &str| args.get(key).and_then(Value::as_i64);
        let b = |key: &str| args.get(key).and_then(Value::as_bool);
        let need_s = |key: &str| s(key).ok_or_else(|| anyhow!("missing `{key}`"));
        let need_i = |key: &str| i(key).ok_or_else(|| anyhow!("missing `{key}`"));
        // `null` clears, absent leaves unchanged.
        let opt = |key: &str| -> Option<Option<String>> {
            args.get(key).map(|v| v.as_str().map(str::to_string).filter(|s| !s.is_empty()))
        };

        match cmd {
            "snapshot" => Ok(serde_json::to_value(self.snapshot())?),

            "reply" => {
                self.reply(need_i("thread_id")?, need_s("body")?, s("attention"), b("handle").unwrap_or(false), i("reply_to"))
                    .await
            }
            "compose" => {
                self.compose(
                    need_s("to")?,
                    s("subject"),
                    need_s("body")?,
                    s("attention").unwrap_or_else(|| "next-checkpoint".into()),
                    b("requires_disposition").unwrap_or(false),
                    s("from"),
                )
                .await
            }
            "outbox_retry" => self.dispatch_outbox(&need_s("local_id")?).await,
            "outbox_check" => self.check_outbox(&need_s("local_id")?).await,
            "outbox_discard" => {
                let local_id = need_s("local_id")?;
                self.mutate(|store| {
                    // An uncertain message may have been accepted; only a definite failure is discardable.
                    let before = store.outbox.len();
                    store.outbox.retain(|o| !(o.local_id == local_id && o.state == "failed"));
                    before != store.outbox.len()
                })
                .map(|removed| json!({ "removed": removed }))
            }

            "mark_done" => {
                let errors = self.handle_thread(need_i("thread_id")?, "handled in Station").await?;
                Ok(json!({ "errors": errors }))
            }
            "flag" => {
                let (thread_id, on) = (need_i("thread_id")?, b("on").unwrap_or(true));
                self.mutate(|store| store.threads.entry(thread_id).or_default().pinned = on)?;
                Ok(Value::Null)
            }
            "snooze" => {
                let thread_id = need_i("thread_id")?;
                let until = i("until_ms");
                self.mutate(|store| store.threads.entry(thread_id).or_default().snooze_until_ms = until)?;
                let mut errors: Vec<String> = Vec::new();
                if let (Some(until), Ok(client)) = (until, self.client()) {
                    let messages = {
                        let store = self.store.lock().unwrap();
                        Self::thread_messages(&store, thread_id)
                    };
                    let note = format!("snoozed in Station until {}", iso(until));
                    for m in messages.iter().filter(|m| m.open_obligation()) {
                        if let Err(e) = self.record_disposition(&client, m, "deferred", Some(&note)).await {
                            errors.push(format!("#{}: {e:#}", m.id));
                        }
                    }
                }
                Ok(json!({ "errors": errors }))
            }
            "disposition" => {
                let client = self.client()?;
                let id = need_i("message_id")?;
                let message = self
                    .store
                    .lock()
                    .unwrap()
                    .messages
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| anyhow!("unknown message"))?;
                self.record_disposition(&client, &message, &need_s("state")?, s("note").as_deref()).await?;
                Ok(Value::Null)
            }
            "mark_read" => {
                let thread_id = need_i("thread_id")?;
                let (to_ack, receipts) = self.mutate(|store| {
                    let messages = Self::thread_messages(store, thread_id);
                    let top = messages
                        .iter()
                        .filter(|m| m.direction == Direction::Inbound)
                        .map(|m| m.id)
                        .max()
                        .unwrap_or(0);
                    let local = store.threads.entry(thread_id).or_default();
                    local.read_through = local.read_through.max(top);
                    let to_ack: Vec<StoredMessage> =
                        messages.into_iter().filter(|m| m.open_obligation() && m.disposition.is_none()).collect();
                    (to_ack, store.settings.read_receipts)
                })?;
                if receipts {
                    if let Ok(client) = self.client() {
                        for m in &to_ack {
                            // Tells the agent a person has seen it; the obligation stays open.
                            let _ = self.record_disposition(&client, m, "acknowledged", Some("seen by operator in Station")).await;
                        }
                    }
                }
                Ok(Value::Null)
            }
            "mark_unread" => {
                let thread_id = need_i("thread_id")?;
                self.mutate(|store| {
                    let first = store
                        .messages
                        .values()
                        .filter(|m| m.thread_id == thread_id && m.direction == Direction::Inbound)
                        .map(|m| m.id)
                        .max()
                        .unwrap_or(1);
                    store.threads.entry(thread_id).or_default().read_through = first - 1;
                })?;
                Ok(Value::Null)
            }
            "thread_update" => {
                let thread_id = need_i("thread_id")?;
                let project = opt("project_id");
                let muted = b("muted");
                self.mutate(|store| {
                    let local = store.threads.entry(thread_id).or_default();
                    if let Some(p) = project {
                        local.project_id = p;
                    }
                    if let Some(m) = muted {
                        local.muted = m;
                    }
                })?;
                Ok(Value::Null)
            }

            "agent_update" => {
                let address = need_s("address")?;
                let (nickname, notes, locator, project) =
                    (opt("nickname"), opt("notes"), opt("locator_note"), opt("project_id"));
                let notify: Option<NotifyOverride> =
                    args.get("notify").map(|v| serde_json::from_value(v.clone())).transpose()?;
                self.mutate(|store| {
                    let agent = store.agents.entry(address.clone()).or_insert_with(|| AgentRecord {
                        address,
                        first_seen_ms: now_ms(),
                        ..Default::default()
                    });
                    if let Some(v) = nickname {
                        agent.nickname = v;
                    }
                    if let Some(v) = notes {
                        agent.notes = v;
                    }
                    if let Some(v) = locator {
                        agent.locator_note = v;
                    }
                    if let Some(v) = project {
                        agent.project_id = v;
                    }
                    if let Some(v) = notify {
                        agent.notify = v;
                    }
                })?;
                Ok(Value::Null)
            }

            "project_create" => {
                let name = need_s("name")?.trim().to_string();
                if name.is_empty() {
                    bail!("project name is empty");
                }
                let parent = s("parent_id");
                let id = self.mutate(|store| {
                    let mut path = parent.as_deref().map(|p| org::project_path(&store.projects, p)).unwrap_or_default();
                    path.push(name);
                    org::ensure_path(&mut store.projects, &path)
                })?;
                Ok(json!({ "id": id }))
            }
            "project_update" => {
                let id = need_s("id")?;
                let (name, parent, color) = (s("name"), opt("parent_id"), opt("color"));
                let notify: Option<NotifyOverride> =
                    args.get("notify").map(|v| serde_json::from_value(v.clone())).transpose()?;
                self.mutate(|store| {
                    // Refuse to move a folder inside itself.
                    if let Some(Some(new_parent)) = &parent {
                        if org::lineage(&store.projects, new_parent).iter().any(|p| p.id == id) {
                            return Err(anyhow!("cannot move a folder inside itself"));
                        }
                    }
                    let project =
                        store.projects.iter_mut().find(|p| p.id == id).ok_or_else(|| anyhow!("unknown project"))?;
                    if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
                        project.name = n.trim().to_string();
                    }
                    if let Some(p) = parent {
                        project.parent_id = p;
                    }
                    if let Some(c) = color {
                        project.color = c;
                    }
                    if let Some(n) = notify {
                        project.notify = n;
                    }
                    Ok(())
                })??;
                Ok(Value::Null)
            }
            "project_delete" => {
                let id = need_s("id")?;
                self.mutate(|store| {
                    let parent = store.projects.iter().find(|p| p.id == id).and_then(|p| p.parent_id.clone());
                    // Children and filings move up one level; nothing is orphaned.
                    for p in store.projects.iter_mut().filter(|p| p.parent_id.as_deref() == Some(&id)) {
                        p.parent_id = parent.clone();
                    }
                    for a in store.agents.values_mut().filter(|a| a.project_id.as_deref() == Some(&id)) {
                        a.project_id = parent.clone();
                    }
                    for t in store.threads.values_mut().filter(|t| t.project_id.as_deref() == Some(&id)) {
                        t.project_id = parent.clone();
                    }
                    match &parent {
                        Some(parent) => {
                            for r in store.rules.iter_mut().filter(|r| r.project_id == id) {
                                r.project_id = parent.clone();
                            }
                        }
                        None => store.rules.retain(|r| r.project_id != id),
                    }
                    store.projects.retain(|p| p.id != id);
                })?;
                Ok(Value::Null)
            }

            "rule_create" => {
                let field: RuleField = serde_json::from_value(args.get("field").cloned().unwrap_or(Value::Null))?;
                let (pattern, project_id) = (need_s("pattern")?, need_s("project_id")?);
                let id = format!("r-{}", uuid::Uuid::new_v4().simple());
                let rule = Rule { id: id.clone(), field, pattern, project_id, enabled: true };
                self.mutate(|store| store.rules.push(rule))?;
                Ok(json!({ "id": id }))
            }
            "rule_update" => {
                let id = need_s("id")?;
                let field: Option<RuleField> =
                    args.get("field").map(|v| serde_json::from_value(v.clone())).transpose()?;
                let (pattern, project_id, enabled) = (s("pattern"), s("project_id"), b("enabled"));
                self.mutate(|store| {
                    if let Some(rule) = store.rules.iter_mut().find(|r| r.id == id) {
                        if let Some(f) = field {
                            rule.field = f;
                        }
                        if let Some(p) = pattern {
                            rule.pattern = p;
                        }
                        if let Some(p) = project_id {
                            rule.project_id = p;
                        }
                        if let Some(e) = enabled {
                            rule.enabled = e;
                        }
                    }
                })?;
                Ok(Value::Null)
            }
            "rule_delete" => {
                let id = need_s("id")?;
                self.mutate(|store| store.rules.retain(|r| r.id != id))?;
                Ok(Value::Null)
            }
            "rule_move" => {
                let (id, delta) = (need_s("id")?, need_i("delta")?);
                self.mutate(|store| {
                    if let Some(index) = store.rules.iter().position(|r| r.id == id) {
                        let target = (index as i64 + delta).clamp(0, store.rules.len() as i64 - 1) as usize;
                        let rule = store.rules.remove(index);
                        store.rules.insert(target, rule);
                    }
                })?;
                Ok(Value::Null)
            }
            "suggestion_apply" => {
                let id = need_s("id")?;
                let custom_path: Option<Vec<String>> =
                    args.get("project_path").and_then(|v| serde_json::from_value(v.clone()).ok());
                let project = self.mutate(|store| {
                    let suggestion = org::suggestions(store).into_iter().find(|s| s.id == id)?;
                    let path = custom_path.filter(|p| !p.is_empty()).unwrap_or(suggestion.project_path);
                    let project_id = org::ensure_path(&mut store.projects, &path)?;
                    let rule = Rule {
                        id: format!("r-{}", uuid::Uuid::new_v4().simple()),
                        field: suggestion.field,
                        pattern: suggestion.pattern,
                        project_id: project_id.clone(),
                        enabled: true,
                    };
                    // A rule for a subfolder must be checked before a rule for its parent.
                    let ancestors: Vec<String> = org::lineage(&store.projects, &project_id)
                        .into_iter()
                        .map(|p| p.id)
                        .filter(|id| id != &project_id)
                        .collect();
                    match store.rules.iter().position(|r| ancestors.contains(&r.project_id)) {
                        Some(index) => store.rules.insert(index, rule),
                        None => store.rules.push(rule),
                    }
                    Some(project_id)
                })?;
                project.map(|id| json!({ "project_id": id })).ok_or_else(|| anyhow!("suggestion no longer applies"))
            }
            "suggestion_dismiss" => {
                let id = need_s("id")?;
                self.mutate(|store| store.suggestions_dismissed.push(id))?;
                Ok(Value::Null)
            }

            "settings_update" => {
                let mut next: Settings = serde_json::from_value(
                    args.get("settings").cloned().ok_or_else(|| anyhow!("missing `settings`"))?,
                )?;
                self.mutate(|store| {
                    // Attended addresses change only through address_add / address_remove.
                    next.addresses = store.settings.addresses.clone();
                    store.settings = next;
                })?;
                Ok(Value::Null)
            }
            "set_focus" => {
                let mode: FocusMode = serde_json::from_value(args.get("mode").cloned().unwrap_or(Value::Null))?;
                let until = i("minutes").map(|m| now_ms() + m * 60_000);
                self.mutate(|store| store.settings.focus = Focus { mode, until_ms: until })?;
                Ok(Value::Null)
            }
            "address_add" => {
                let address = need_s("address")?.trim().to_string();
                if address.is_empty() {
                    bail!("address is empty");
                }
                let label = s("label");
                let added = self.mutate(|store| {
                    if store.settings.addresses.iter().any(|a| a.address == address) {
                        return false;
                    }
                    store.settings.addresses.push(AttendedAddress { address: address.clone(), label, enabled: true });
                    true
                })?;
                if added {
                    if let Ok(client) = self.client() {
                        self.spawn_receiver(client, address);
                    }
                }
                Ok(json!({ "added": added }))
            }
            "address_remove" => {
                let address = need_s("address")?;
                if let Some(task) = self.tasks.lock().unwrap().remove(&address) {
                    task.abort();
                }
                let mut detach_error = None;
                if let Ok(client) = self.client() {
                    // Deliberate, durable detach: a restart must not resurrect it.
                    if let Err(e) = client.detach(&address).await {
                        detach_error = Some(e.to_string());
                    }
                }
                self.mutate(|store| store.settings.addresses.retain(|a| a.address != address))?;
                self.runtime.lock().unwrap().attached.remove(&address);
                self.health.lock().unwrap().remove(&address);
                Ok(json!({ "detach_error": detach_error }))
            }
            "notification_test" => {
                let tier: Tier = serde_json::from_value(args.get("tier").cloned().unwrap_or(json!("blocked")))?;
                let settings = self.store.lock().unwrap().settings.clone();
                let ctx = policy::Context { tier: Some(tier), ..Default::default() };
                let mut record = policy::decide(
                    &settings,
                    &ctx,
                    "test",
                    None,
                    None,
                    format!("{} · test notification", tier.label()),
                    "This is how this tier reaches you with the current settings.".into(),
                );
                record.kind = "test".into();
                self.publish_notification(record.clone())?;
                Ok(serde_json::to_value(record)?)
            }
            "notifications_clear" => {
                self.mutate(|store| store.notifications.clear())?;
                Ok(Value::Null)
            }
            other => bail!("unknown command `{other}`"),
        }
    }
}

fn push_notification(store: &mut Store, record: NotificationRecord) {
    store.notifications.push(record);
    let extra = store.notifications.len().saturating_sub(400);
    if extra > 0 {
        store.notifications.drain(0..extra);
    }
}

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| ms.to_string())
}

pub fn default_data_dir() -> PathBuf {
    dirs_home().join(".telex-station")
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| Path::new(".").to_path_buf())
}
