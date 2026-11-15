//! Station-local data model: the durable local projection plus the view types
//! handed to the UI. Nothing here is a Telex contract; Telex facts are copied in
//! from the Application Client and kept distinct from self-reported detail.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Human-facing priority tier. Ordered so `max()` picks the most urgent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tier {
    Fyi,
    Routine,
    NeedsReply,
    Blocked,
}

impl Tier {
    /// Telex attention is the only input: Station never infers urgency from text.
    pub fn from_attention(attention: &str) -> Tier {
        match attention {
            "interrupt" => Tier::Blocked,
            "next-checkpoint" => Tier::NeedsReply,
            "fyi" => Tier::Fyi,
            _ => Tier::Routine,
        }
    }
    pub fn attention(self) -> &'static str {
        match self {
            Tier::Blocked => "interrupt",
            Tier::NeedsReply => "next-checkpoint",
            Tier::Routine => "background",
            Tier::Fyi => "fyi",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Tier::Blocked => "Blocked",
            Tier::NeedsReply => "Needs reply",
            Tier::Routine => "Routine",
            Tier::Fyi => "FYI",
        }
    }
    pub const ALL: [Tier; 4] = [Tier::Blocked, Tier::NeedsReply, Tier::Routine, Tier::Fyi];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    Inbound,
    Outbound,
}

/// Self-reported presentation card. Always shown as unverified.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Card {
    pub name: Option<String>,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub host: Option<String>,
    pub os: Option<String>,
    pub cwd: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub session_id: Option<String>,
    pub session_name: Option<String>,
    pub terminal: Option<String>,
    pub open_command: Option<String>,
    pub project_hint: Option<String>,
    pub url: Option<String>,
    pub tags: Vec<String>,
}

impl Card {
    pub fn is_empty(&self) -> bool {
        self == &Card::default()
    }
    /// Newer non-empty fields win; absent fields keep the last known value.
    pub fn merge_from(&mut self, newer: &Card) {
        macro_rules! take {
            ($($f:ident),*) => { $( if newer.$f.is_some() { self.$f = newer.$f.clone(); } )* };
        }
        take!(
            name, harness, model, host, os, cwd, repo, branch, session_id, session_name, terminal,
            open_command, project_hint, url
        );
        if !newer.tags.is_empty() {
            self.tags = newer.tags.clone();
        }
    }
}

/// Exact delivery identity as supplied by the Application Client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryRef {
    pub logical_store_id: String,
    pub message_id: i64,
    pub recipient: String,
    pub delivery_id: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Receipt {
    /// Was the recipient address occupied when Telex accepted the message?
    pub occupied_at_send: Option<bool>,
    /// Exact-recipient consumption (the session actually took the delivery).
    pub consumed: bool,
    /// Latest workflow disposition recorded by the recipient, if any.
    pub disposition: Option<String>,
    pub refreshed_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredMessage {
    pub id: i64,
    pub thread_id: i64,
    pub parent_id: Option<i64>,
    pub direction: Direction,
    pub from: Option<String>,
    pub to: String,
    #[serde(default)]
    pub cc: Vec<String>,
    pub kind: String,
    pub attention: String,
    pub requires_disposition: bool,
    pub subject: Option<String>,
    pub body: String,
    pub metadata: Option<String>,
    pub sent_at_ms: i64,
    pub ingested_at_ms: i64,
    /// "primary" or "cc" for inbound; "author" for outbound.
    pub role: String,
    /// Which attended address this row belongs to.
    pub station_address: String,
    pub delivery: Option<DeliveryRef>,
    /// Delivery acknowledged to Telex (only after this row was persisted).
    #[serde(default)]
    pub acked: bool,
    /// Our latest disposition on an inbound message.
    #[serde(default)]
    pub disposition: Option<String>,
    /// Outbound only.
    #[serde(default)]
    pub receipt: Option<Receipt>,
    /// Outbound only: serialized Application Client `RecoveryHandle`.
    #[serde(default)]
    pub recovery: Option<serde_json::Value>,
    /// How this row reached the projection: "receive" | "history" | "authored".
    #[serde(default)]
    pub source: String,
}

impl StoredMessage {
    pub fn tier(&self) -> Tier {
        Tier::from_attention(&self.attention)
    }
    pub fn is_terminal_disposition(&self) -> bool {
        matches!(
            self.disposition.as_deref(),
            Some("handled") | Some("rejected") | Some("closed")
        )
    }
    pub fn open_obligation(&self) -> bool {
        self.direction == Direction::Inbound
            && self.role == "primary"
            && self.requires_disposition
            && !self.is_terminal_disposition()
    }
}

/// An authored message that Telex has not (yet) accepted.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutboxItem {
    pub local_id: String,
    pub operation_id: String,
    pub thread_id: Option<i64>,
    pub reply_to: Option<i64>,
    pub sender: String,
    pub to: String,
    pub subject: Option<String>,
    pub body: String,
    pub attention: String,
    pub requires_disposition: bool,
    pub metadata: Option<String>,
    pub handle_on_accept: bool,
    pub created_at_ms: i64,
    /// "sending" | "failed" | "indeterminate"
    pub state: String,
    pub error: Option<String>,
    pub recovery: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ThreadLocal {
    /// Highest message id the human has seen in this thread.
    pub read_through: i64,
    /// Everything up to this message id is resolved from the human's side.
    pub done_through: i64,
    pub snooze_until_ms: Option<i64>,
    /// Explicit filing; overrides agent filing and rules.
    pub project_id: Option<String>,
    pub pinned: bool,
    pub muted: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NotifyOverride {
    #[default]
    Inherit,
    /// Never interrupt; items still land in the feed and count in badges.
    Mute,
    /// Treat everything non-FYI from here as at least "Needs reply".
    Elevate,
    /// Only Blocked breaks through.
    BlockedOnly,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentRecord {
    pub address: String,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    /// Latest merged self-reported card.
    pub card: Card,
    pub nickname: Option<String>,
    pub notes: Option<String>,
    /// Local pinned "where to find it" note that outranks the self-report.
    pub locator_note: Option<String>,
    pub project_id: Option<String>,
    pub notify: NotifyOverride,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub notify: NotifyOverride,
    #[serde(default)]
    pub order: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuleField {
    Address,
    Repo,
    Host,
    Harness,
    Branch,
    Tag,
    Hint,
    Name,
}

/// "When <field> matches <pattern>, file the agent in <project>". Patterns are
/// case-insensitive globs (`*` wildcard).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub field: RuleField,
    pub pattern: String,
    pub project_id: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttendedAddress {
    pub address: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelPolicy {
    /// OS notification banner.
    pub banner: bool,
    /// Play a sound with the banner.
    pub sound: bool,
    /// Demand attention (dock bounce / persistent in-app alert strip).
    pub demand: bool,
    /// Count toward the dock / menu-bar badge.
    pub badge: bool,
    /// Re-notify while still unanswered. 0 disables.
    pub remind_after_min: u32,
}

impl Default for ChannelPolicy {
    fn default() -> Self {
        ChannelPolicy { banner: false, sound: false, demand: false, badge: true, remind_after_min: 0 }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FocusMode {
    #[default]
    Off,
    /// Only Blocked interrupts.
    BlockedOnly,
    /// Nothing interrupts; badges still update.
    Silent,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Focus {
    pub mode: FocusMode,
    pub until_ms: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuietHours {
    pub enabled: bool,
    /// Minutes after local midnight.
    pub start_min: u32,
    pub end_min: u32,
    /// Blocked still breaks through quiet hours when true.
    pub allow_blocked: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub operator_name: String,
    pub addresses: Vec<AttendedAddress>,
    pub blocked: ChannelPolicy,
    pub needs_reply: ChannelPolicy,
    pub routine: ChannelPolicy,
    pub fyi: ChannelPolicy,
    pub focus: Focus,
    pub quiet_hours: QuietHours,
    /// Record an `acknowledged` disposition when the human opens a message that
    /// requires disposition, so the agent can see it was seen by a person.
    pub read_receipts: bool,
    /// Non-critical banners arriving within this window collapse into one.
    pub coalesce_window_s: u32,
    /// File agents by their self-reported project hint when it names an existing project.
    pub use_project_hints: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            operator_name: "operator".into(),
            addresses: Vec::new(),
            blocked: ChannelPolicy { banner: true, sound: true, demand: true, badge: true, remind_after_min: 10 },
            needs_reply: ChannelPolicy { banner: true, sound: false, demand: false, badge: true, remind_after_min: 0 },
            routine: ChannelPolicy { banner: false, sound: false, demand: false, badge: true, remind_after_min: 0 },
            fyi: ChannelPolicy { banner: false, sound: false, demand: false, badge: false, remind_after_min: 0 },
            focus: Focus::default(),
            quiet_hours: QuietHours { enabled: false, start_min: 22 * 60, end_min: 7 * 60, allow_blocked: true },
            read_receipts: true,
            coalesce_window_s: 8,
            use_project_hints: true,
        }
    }
}

impl Settings {
    pub fn policy(&self, tier: Tier) -> &ChannelPolicy {
        match tier {
            Tier::Blocked => &self.blocked,
            Tier::NeedsReply => &self.needs_reply,
            Tier::Routine => &self.routine,
            Tier::Fyi => &self.fyi,
        }
    }
}

/// Retained evidence of one notification decision. A "shown" record means the
/// Station submitted it, never that a human saw it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotificationRecord {
    pub id: String,
    pub at_ms: i64,
    /// "new" | "reminder" | "summary" | "test"
    pub kind: String,
    pub thread_id: Option<i64>,
    pub message_id: Option<i64>,
    pub tier: Tier,
    pub effective_tier: Tier,
    pub title: String,
    pub body: String,
    pub banner: bool,
    pub sound: bool,
    pub demand: bool,
    pub badge: bool,
    /// "shown" | "quiet" | "suppressed"
    pub outcome: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub version: u32,
    pub messages: BTreeMap<i64, StoredMessage>,
    pub outbox: Vec<OutboxItem>,
    pub threads: BTreeMap<i64, ThreadLocal>,
    pub agents: BTreeMap<String, AgentRecord>,
    pub projects: Vec<Project>,
    pub rules: Vec<Rule>,
    pub settings: Settings,
    pub notifications: Vec<NotificationRecord>,
    /// Per attended address: highest message id seen through history sync.
    pub history_cursor: BTreeMap<String, i64>,
    /// Last reminder per thread, to pace re-notification.
    pub reminded_at_ms: BTreeMap<i64, i64>,
    pub suggestions_dismissed: Vec<String>,
}

// ---------------------------------------------------------------- view types

#[derive(Clone, Debug, Serialize)]
pub struct MessageView {
    pub id: i64,
    pub local_id: Option<String>,
    pub direction: Direction,
    pub from: Option<String>,
    pub to: String,
    pub cc: Vec<String>,
    pub kind: String,
    pub attention: String,
    pub tier: Tier,
    pub requires_disposition: bool,
    pub subject: Option<String>,
    pub body: String,
    pub sent_at_ms: i64,
    pub role: String,
    pub disposition: Option<String>,
    pub open_obligation: bool,
    pub unread: bool,
    pub card: Option<Card>,
    pub replies: Vec<String>,
    /// Opaque metadata Station does not interpret, shown inertly on demand.
    pub other_metadata: Option<String>,
    pub receipt: Option<Receipt>,
    /// Outbound: "sending" | "accepted" | "failed" | "indeterminate"
    pub state: Option<String>,
    pub error: Option<String>,
    pub source: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ThreadView {
    pub thread_id: i64,
    pub subject: String,
    /// The counterpart address (the agent).
    pub agent: String,
    pub station_address: String,
    /// "needs-me" | "waiting" | "snoozed" | "done" | "fyi"
    pub status: String,
    pub tier: Tier,
    pub unread: u32,
    pub open_obligations: u32,
    pub last_activity_ms: i64,
    pub last_inbound_ms: i64,
    pub snippet: String,
    pub project_id: Option<String>,
    /// "thread" | "agent" | "rule:<id>" | "hint" | "none"
    pub project_source: String,
    pub snooze_until_ms: Option<i64>,
    pub pinned: bool,
    pub muted: bool,
    pub cc_only: bool,
    pub messages: Vec<MessageView>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Presence {
    /// "listening" | "attached" | "offline" | "unknown"
    pub state: String,
    pub detail: Option<String>,
    /// Telex-recorded host of the current occupant, when known.
    pub host: Option<String>,
    pub description: Option<String>,
    pub tags: Option<String>,
    pub checked_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentView {
    pub address: String,
    pub display_name: String,
    pub card: Card,
    pub nickname: Option<String>,
    pub notes: Option<String>,
    pub locator_note: Option<String>,
    pub project_id: Option<String>,
    pub project_source: String,
    pub notify: NotifyOverride,
    pub presence: Presence,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    pub thread_ids: Vec<i64>,
    pub needs_me: u32,
    pub top_tier: Option<Tier>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Suggestion {
    pub id: String,
    /// "repo" | "namespace" | "host" | "hint"
    pub basis: String,
    pub title: String,
    pub project_path: Vec<String>,
    pub field: RuleField,
    pub pattern: String,
    pub agents: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct HealthView {
    pub address: String,
    pub registered: bool,
    pub receive_ready: bool,
    pub sender_ready: bool,
    pub degraded: bool,
    pub recovering: bool,
    pub pending_unconsumed: i64,
    pub inbound_actionable: i64,
    pub principal: Option<String>,
    pub principal_verification: String,
    pub last_error: Option<String>,
    pub last_receive_ms: Option<i64>,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Counts {
    pub needs_me: u32,
    pub blocked: u32,
    pub needs_reply: u32,
    pub routine: u32,
    pub fyi_unread: u32,
    pub waiting: u32,
    pub snoozed: u32,
    pub unsorted_agents: u32,
    pub badge: u32,
    pub top_tier: Option<Tier>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StationInfo {
    pub operator_name: String,
    pub store_id: String,
    pub backend: String,
    pub connected: bool,
    pub connect_error: Option<String>,
    pub telex_build: Option<String>,
    pub sandbox: bool,
    /// "application-client" | "cli"
    pub port: String,
    pub port_note: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub rev: u64,
    pub now_ms: i64,
    pub station: StationInfo,
    pub threads: Vec<ThreadView>,
    pub agents: Vec<AgentView>,
    pub projects: Vec<Project>,
    pub rules: Vec<Rule>,
    pub settings: Settings,
    pub health: Vec<HealthView>,
    pub notifications: Vec<NotificationRecord>,
    pub suggestions: Vec<Suggestion>,
    pub counts: Counts,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum StationEvent {
    Changed { rev: u64 },
    Notify { record: NotificationRecord },
    Badge { counts: Counts },
}
