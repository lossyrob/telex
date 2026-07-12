//! Command-line surface (clap) and dispatch. Global options resolve backend, db path,
//! default address, and output format; each subcommand maps to a handler in `commands`.

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;

use crate::backend::Backend;
use crate::config::Config;
use crate::daemon_ipc::{WatchPidRole, WatchPidSpec};
use crate::model::Attention;
use crate::output::Format;
use crate::profiles::BackendProfile;

#[derive(Parser)]
#[command(
    name = "telex",
    version,
    about = "A CLI-first message fabric for AI agent sessions",
    long_about = "Telex lets ephemeral agent sessions attach to durable addresses, exchange \
typed operational messages with answerback liveness, and leave an auditable disposition record. \
Run `telex skill` to load agent usage instructions for this build."
)]
pub struct Cli {
    /// Configured backend to use, by name (default: the configured default backend).
    #[arg(long, global = true, env = "TELEX_BACKEND")]
    pub backend: Option<String>,

    /// Override the SQLite path for this invocation (sqlite backends only).
    #[arg(long, global = true, env = "TELEX_DB")]
    pub db: Option<String>,

    /// Address to operate on (default for commands that act on one address).
    #[arg(long, global = true, env = "TELEX_ADDRESS")]
    pub address: Option<String>,

    /// Force JSON output.
    #[arg(long, global = true)]
    pub json: bool,

    /// Force concise text output.
    #[arg(long, global = true)]
    pub text: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Initialize ~/.telex and the backend schema.
    Init,
    /// Show config, backend, address, station/occupancy status.
    Status,
    /// Show telex binary, launcher, protocol, and install metadata.
    Version(VersionArgs),
    /// Install a versioned telex binary and optionally switch current to it.
    Upgrade(UpgradeArgs),
    /// Switch current back to a previously installed version.
    Rollback(RollbackArgs),
    /// Garbage-collect old installed telex versions.
    Gc(GcArgs),
    /// Print the agent usage skill (how to use telex) for this build.
    Skill(SkillArgs),

    /// Attach this session to an address and exit.
    Attach(AttachArgs),
    /// Detach this session's address membership.
    Detach(DetachArgs),
    /// Station lifecycle operations.
    #[command(subcommand)]
    Station(StationCmd),

    /// Block until an actionable message arrives, print it as JSON, and exit.
    Wait(WaitArgs),
    /// List actionable and recent messages for an address.
    Inbox(InboxArgs),
    /// Read a message (optionally with thread context).
    Read(ReadArgs),

    /// Send a message to an address.
    Send(SendArgs),
    /// Reply to a message; threads under it.
    Reply(ReplyArgs),

    /// Acknowledge a message.
    Ack(DispArgs),
    /// Mark a message handled (terminal).
    Handle(DispArgs),
    /// Defer a message.
    Defer(DispArgs),
    /// Reject a message (terminal).
    Reject(DispArgs),
    /// Close a message/thread (terminal).
    Close(DispArgs),
    /// Escalate a message.
    Escalate(DispArgs),

    /// Address directory operations.
    #[command(subcommand)]
    Address(AddressCmd),
    /// Resolve target address(es) by description match or tag.
    Resolve(ResolveArgs),

    /// Manage configured backends (named profiles in ~/.telex/config.toml).
    #[command(subcommand)]
    Backend(BackendCmd),

    /// Hidden Copilot CLI plugin adapter commands.
    #[command(hide = true, subcommand)]
    Copilot(CopilotCmd),

    /// Hidden daemon lifecycle and diagnostics entrypoint.
    #[command(hide = true, subcommand)]
    Daemon(DaemonCmd),

    /// Export messages and disposition history as JSON lines.
    Export(ExportArgs),
}

#[derive(Args)]
pub struct AttachArgs {
    /// One-line directory description of what this session is doing.
    #[arg(long)]
    pub description: Option<String>,
    /// Project/workstream scope this address belongs to.
    #[arg(long)]
    pub scope: Option<String>,
    /// Comma-separated coarse tags (e.g. issue:215,repo:telex).
    #[arg(long)]
    pub tags: Option<String>,
    /// Deprecated compatibility flag; the daemon owns lease heartbeat cadence.
    #[arg(long, default_value_t = 5)]
    pub heartbeat_secs: u64,
    /// Deprecated compatibility flag; the daemon owns backend polling.
    #[arg(long, default_value_t = 1)]
    pub poll_secs: u64,
    /// Deprecated compatibility flag; daemon waiters use daemon IPC frames.
    #[arg(long, default_value_t = 3)]
    pub keepalive_secs: u64,
    /// Occupant identity recorded on the lease (default: session host/pid).
    #[arg(long)]
    pub occupant: Option<String>,
    /// Stable session identity for daemon membership.
    #[arg(long, env = "TELEX_SESSION_ID")]
    pub session: Option<String>,
    /// Deprecated compatibility flag; daemon delivery owns push/poll behavior.
    #[arg(long)]
    pub push: bool,
    /// Back-compat watch pid. Converted to an anchor watch-pid for daemon liveness.
    #[arg(long)]
    pub session_pid: Option<u32>,
    /// Watch a pid as a typed liveness predicate. Accepts PID, anchor:PID, required:PID,
    /// PID:anchor, or PID:required. Repeat to add multiple watch pids.
    #[arg(long, value_parser = parse_watch_pid)]
    pub watch_pid: Vec<WatchPidSpec>,
    /// Deprecated compatibility flag; daemon liveness cadence is internal.
    #[arg(long, default_value_t = 2)]
    pub session_poll_secs: u64,
    /// Do not convert `$TELEX_SESSION_PID` into a daemon watch-pid.
    #[arg(long)]
    pub no_session_bind: bool,
    /// Programmatic-only: harness-neutral on-deliver handler argv registered with the daemon.
    /// Not a CLI flag; set by the Copilot bridge bind path.
    #[arg(skip)]
    pub on_deliver: Option<Vec<String>>,
    /// Programmatic-only: replace the existing on-deliver handler during member refresh.
    #[arg(skip)]
    pub replace_on_deliver: bool,
    /// Programmatic-only: opt an on-deliver push handler into live CC observer traffic.
    #[arg(skip)]
    pub on_deliver_wake_on_cc: bool,
}

#[derive(Args)]
pub struct WaitArgs {
    /// Stable session identity for daemon membership.
    #[arg(long, env = "TELEX_SESSION_ID")]
    pub session: Option<String>,
    /// Give up waiting after this many milliseconds (exit code 2); default is no idle timeout.
    #[arg(long)]
    pub timeout_ms: Option<u64>,
    /// Only wake for messages at this attention or higher priority.
    #[arg(long, value_parser = parse_attention_arg)]
    pub min_attention: Option<Attention>,
    /// Also wake for live CC traffic without making CC ack-required.
    #[arg(long)]
    pub wake_on_cc: bool,
    /// Resume delivery strictly after this message id.
    #[arg(long, default_value_t = 0)]
    pub since: i64,
    /// Deprecated idle-wait compatibility watchdog. For daemon waits, only applies after timeout-ms.
    #[arg(long, default_value_t = 8_000)]
    pub hang_ms: u64,
    /// Retry daemon reconnect/re-register for this long after EOF/restart (ms).
    #[arg(long, env = "TELEX_RECONNECT_GRACE_MS")]
    pub reconnect_grace_ms: Option<u64>,
    /// Holder DB-heartbeat age beyond which it is considered degraded (ms).
    #[arg(long, default_value_t = 15_000)]
    pub stale_heartbeat_ms: i64,
    /// Write outcome artifacts into this directory so a detached, variable-free
    /// invocation can deliver results without relying on captured stdout. Writes
    /// `message.json` (on delivery), `status.json` (always), and `exit.code`
    /// (always, written last as the completion marker).
    #[arg(long)]
    pub out_dir: Option<PathBuf>,
}

#[derive(Args)]
pub struct DetachArgs {
    /// Stable session identity for daemon membership.
    #[arg(long, env = "TELEX_SESSION_ID")]
    pub session: Option<String>,
}

#[derive(Subcommand)]
pub enum StationCmd {
    /// Show this session's attended addresses and waiter state.
    Status(StationStatusArgs),
    /// Stop this session's station: release membership and drain its live waiters.
    Stop(StationStopArgs),
}

#[derive(Args)]
pub struct StationStatusArgs {
    /// Stable session identity for daemon membership.
    #[arg(long, env = "TELEX_SESSION_ID")]
    pub session: Option<String>,
    /// Show all stations in the selected store instead of only this session.
    #[arg(long)]
    pub all_sessions: bool,
}

#[derive(Args)]
pub struct StationStopArgs {
    /// Stable session identity for daemon membership.
    #[arg(long, env = "TELEX_SESSION_ID")]
    pub session: Option<String>,
    /// How long to wait for live waiter processes to exit after teardown is signaled (ms).
    #[arg(long, default_value_t = 3_000)]
    pub wait_grace_ms: u64,
}

#[derive(Args)]
pub struct InboxArgs {
    /// Include all recent messages, not just actionable ones.
    #[arg(long)]
    pub all: bool,
    /// Maximum messages to list.
    #[arg(long, default_value_t = 50)]
    pub limit: i64,
}

#[derive(Args)]
pub struct ReadArgs {
    /// Message id to read.
    #[arg(long)]
    pub id: i64,
    /// Include compact thread context.
    #[arg(long)]
    pub thread: bool,
    /// Include full thread history and dispositions.
    #[arg(long)]
    pub full: bool,
}

#[derive(Args)]
pub struct SendArgs {
    /// Destination address.
    #[arg(long)]
    pub to: String,
    /// Subject line.
    #[arg(long)]
    pub subject: Option<String>,
    /// Message body (inline). Body/subject/metadata are capped below the 1 MiB IPC frame.
    #[arg(long)]
    pub body: Option<String>,
    /// Read the message body from a UTF-8 file (`-` = stdin); capped below the 1 MiB IPC frame.
    #[arg(long)]
    pub body_file: Option<String>,
    /// Read the message body from stdin (UTF-8). Equivalent to `--body-file -`. On Windows /
    /// PowerShell, run `$OutputEncoding = [System.Text.Encoding]::UTF8` before piping
    /// non-ASCII content, or write a UTF-8 file and use `--body-file <path>` instead.
    #[arg(long)]
    pub body_stdin: bool,
    /// CC addresses (visible observers). May be repeated and/or comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub cc: Vec<String>,
    /// Message kind/profile label.
    #[arg(long, default_value = "note")]
    pub kind: String,
    /// Attention level: interrupt | next-checkpoint | background | fyi.
    #[arg(long, default_value = "background")]
    pub attention: String,
    /// Mark that the recipient must disposition this message.
    #[arg(long)]
    pub requires_disposition: bool,
    /// Sender address (defaults to the global --address if set).
    #[arg(long)]
    pub from: Option<String>,
    /// Arbitrary JSON metadata; counted with body/subject against the IPC payload cap.
    #[arg(long)]
    pub metadata: Option<String>,
    /// Stable session identity for daemon membership.
    #[arg(long, env = "TELEX_SESSION_ID")]
    pub session: Option<String>,
}

#[derive(Args)]
pub struct ReplyArgs {
    /// The message id being replied to.
    #[arg(long)]
    pub to_message: i64,
    /// Reply body (inline). Body/subject are capped below the 1 MiB IPC frame.
    #[arg(long)]
    pub body: Option<String>,
    /// Read the reply body from a UTF-8 file (`-` = stdin); capped below the 1 MiB IPC frame.
    #[arg(long)]
    pub body_file: Option<String>,
    /// Read the reply body from stdin (UTF-8). Equivalent to `--body-file -`. On Windows /
    /// PowerShell, run `$OutputEncoding = [System.Text.Encoding]::UTF8` before piping
    /// non-ASCII content, or write a UTF-8 file and use `--body-file <path>` instead.
    #[arg(long)]
    pub body_stdin: bool,
    /// Subject (defaults to "Re: <parent subject>").
    #[arg(long)]
    pub subject: Option<String>,
    /// CC addresses (visible observers). May be repeated and/or comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub cc: Vec<String>,
    /// Attention level.
    #[arg(long, default_value = "background")]
    pub attention: String,
    /// Mark that the recipient must disposition this reply.
    #[arg(long)]
    pub requires_disposition: bool,
    /// Sender address (defaults to the global --address if set).
    #[arg(long)]
    pub from: Option<String>,
    /// Message kind/profile label.
    #[arg(long, default_value = "note")]
    pub kind: String,
    /// Stable session identity for daemon membership.
    #[arg(long, env = "TELEX_SESSION_ID")]
    pub session: Option<String>,
}

#[derive(Args)]
pub struct DispArgs {
    /// Message id to disposition.
    #[arg(long)]
    pub id: i64,
    /// Optional note recorded with the disposition.
    #[arg(long)]
    pub note: Option<String>,
    /// Recipient address whose disposition this is (defaults to the message's to_addr).
    #[arg(long)]
    pub recipient: Option<String>,
    /// Stable session identity for daemon membership.
    #[arg(long, env = "TELEX_SESSION_ID")]
    pub session: Option<String>,
}

#[derive(Subcommand)]
pub enum AddressCmd {
    /// List addresses with description, occupancy, and liveness.
    List(AddressListArgs),
    /// Show detail for one address (uses --address).
    Show,
    /// Retire an address (drops from normal listings).
    Retire,
}

#[derive(Args)]
pub struct AddressListArgs {
    /// Limit to addresses in this scope.
    #[arg(long)]
    pub scope: Option<String>,
    /// Substring match against address or description.
    #[arg(long)]
    pub r#match: Option<String>,
    /// Match a tag (substring of the tags field).
    #[arg(long)]
    pub tag: Option<String>,
    /// Include retired addresses.
    #[arg(long)]
    pub all: bool,
}

#[derive(Args)]
pub struct ResolveArgs {
    /// Substring to match against address or description.
    #[arg(long)]
    pub r#match: Option<String>,
    /// Tag to match.
    #[arg(long)]
    pub tag: Option<String>,
    /// Limit to a scope.
    #[arg(long)]
    pub scope: Option<String>,
}

#[derive(Args)]
pub struct ExportArgs {
    /// Limit to messages to/from this address (defaults to the global --address).
    #[arg(long)]
    pub address: Option<String>,
    /// Limit to a thread id.
    #[arg(long)]
    pub thread: Option<i64>,
    /// Only messages with id greater than this.
    #[arg(long, default_value_t = 0)]
    pub since: i64,
}

#[derive(Subcommand)]
pub enum DaemonCmd {
    /// Run the daemon server loop.
    Serve,
    /// Show daemon singleton status.
    Status,
    /// Show daemon/protocol version metadata.
    Version,
    /// Mark one station idle without destroying membership or buffered deliveries.
    Reset(DaemonResetArgs),
    /// Mark all stations for a session idle without destroying membership or buffered deliveries.
    SessionEnd(DaemonSessionEndArgs),
    /// Stop the daemon.
    Stop(DaemonStopArgs),
}

#[derive(Args)]
pub struct DaemonStopArgs {
    /// Drain in-flight work before exiting.
    #[arg(long)]
    pub drain: bool,
}

#[derive(Args)]
pub struct DaemonResetArgs {
    /// Address to mark idle (defaults to global --address).
    #[arg(long)]
    pub address: Option<String>,
}

#[derive(Args)]
pub struct DaemonSessionEndArgs {
    /// Stable session identity to mark ended.
    #[arg(long, env = "TELEX_SESSION_ID")]
    pub session: Option<String>,
}

#[derive(Subcommand)]
pub enum CopilotCmd {
    /// Register a Copilot session using Copilot env vars mapped to generic telex inputs.
    Attach(CopilotAttachArgs),
    /// Re-provision this Copilot session's push bridge and re-register the address after resume.
    #[command(alias = "repair")]
    Resume(CopilotResumeArgs),
    /// Handle Copilot sessionEnd by non-destructively ending this session in the daemon.
    #[command(hide = true)]
    SessionEnd(CopilotSessionEndArgs),
    /// Handle Copilot agentStop by nudging unarmed attended stations to re-arm or detach.
    #[command(hide = true)]
    TurnGuard(CopilotTurnGuardArgs),
    /// Print version-matched Copilot CLI instructions (the binary is the source of truth).
    Skill(CopilotSkillArgs),
    /// Deliver one telex message (descriptor on stdin) into a session via its bridge.
    #[command(hide = true)]
    Push(CopilotPushArgs),
    /// Handle Copilot agentStop by draining messages deferred while the session was busy.
    #[command(hide = true)]
    Drain(CopilotDrainArgs),
    /// Detach a Copilot session's address and tear down its bridge if it was the last binding.
    Detach(CopilotDetachArgs),
    /// Prepare or execute the single-shot pull fallback used when extension push is unavailable.
    #[command(subcommand)]
    Fallback(CopilotFallbackCmd),
    /// Garbage-collect stale Copilot bridge files for unloaded sessions.
    Gc(CopilotGcArgs),
}

#[derive(Args)]
pub struct VersionArgs {
    /// Inspect a specific versioned install root instead of inferring it from this executable.
    #[arg(long)]
    pub root: Option<PathBuf>,
}

#[derive(Args)]
pub struct UpgradeArgs {
    /// Local telex binary or directory containing telex(.exe) to install (manual/local
    /// upgrade path). Omit to discover, download, verify, and install the latest
    /// compatible public GitHub release.
    #[arg(long = "from", value_name = "PATH")]
    pub from: Option<PathBuf>,
    /// Release tag to install/switch to. Without --from this selects an explicit public
    /// release (e.g. v0.2.0); with --from it labels the local install (defaults to this
    /// binary's package version).
    #[arg(long)]
    pub version: Option<String>,
    /// Reinstall/switch even when the resolved release is already the current version.
    #[arg(long)]
    pub force: bool,
    /// GitHub repository (owner/name) to fetch releases from. Hidden: for tests and
    /// enterprise mirrors only — changing it changes which source telex trusts.
    #[arg(
        long,
        hide = true,
        env = "TELEX_UPGRADE_REPO",
        default_value = "lossyrob/telex"
    )]
    pub repo: String,
    /// Versioned install root (default: inferred install root or platform default).
    #[arg(long)]
    pub root: Option<PathBuf>,
    /// Install into versions/<tag> but do not switch current.
    #[arg(long)]
    pub no_switch: bool,
    /// Skip daemon drain before switching current (not recommended).
    #[arg(long)]
    pub skip_drain: bool,
    /// Bound daemon drain before switching current.
    #[arg(long, default_value_t = 10_000)]
    pub drain_timeout_ms: u64,
}

#[derive(Args)]
pub struct RollbackArgs {
    /// Installed version tag to switch to (default: previous).
    #[arg(long)]
    pub version: Option<String>,
    /// Versioned install root (default: inferred install root or platform default).
    #[arg(long)]
    pub root: Option<PathBuf>,
    /// Skip daemon drain before switching current (not recommended).
    #[arg(long)]
    pub skip_drain: bool,
    /// Bound daemon drain before switching current.
    #[arg(long, default_value_t = 10_000)]
    pub drain_timeout_ms: u64,
}

#[derive(Args)]
pub struct GcArgs {
    /// Versioned install root (default: inferred install root or platform default).
    #[arg(long)]
    pub root: Option<PathBuf>,
    /// Report what would be removed without deleting anything.
    #[arg(long)]
    pub dry_run: bool,
    /// Try removal of stale versions even after ordinary removal errors; never removes current,
    /// previous, or the active process version.
    #[arg(long)]
    pub force: bool,
}

#[derive(Args)]
pub struct CopilotAttachArgs {
    /// Stable Copilot session identity; defaults to COPILOT_AGENT_SESSION_ID.
    #[arg(long)]
    pub session: Option<String>,
    /// One-line directory description of what this session is doing.
    #[arg(long)]
    pub description: Option<String>,
    /// Project/workstream scope this address belongs to.
    #[arg(long)]
    pub scope: Option<String>,
    /// Comma-separated coarse tags (e.g. issue:215,repo:telex).
    #[arg(long)]
    pub tags: Option<String>,
    /// Occupant identity recorded on the lease (default: session host/pid).
    #[arg(long)]
    pub occupant: Option<String>,
    /// Provision the in-session push bridge (write the extension and register the
    /// on-deliver push handler) so messages arrive as turns without a waiter.
    #[arg(long)]
    pub copilot_bridge: bool,
    /// With --copilot-bridge, push live CC observer traffic to this session.
    #[arg(long)]
    pub wake_on_cc: bool,
}

#[derive(Args)]
pub struct CopilotResumeArgs {
    /// Stable Copilot session identity; defaults to COPILOT_AGENT_SESSION_ID.
    #[arg(long)]
    pub session: Option<String>,
    /// One-line directory description of what this session is doing.
    #[arg(long)]
    pub description: Option<String>,
    /// Project/workstream scope this address belongs to.
    #[arg(long)]
    pub scope: Option<String>,
    /// Comma-separated coarse tags (e.g. issue:215,repo:telex).
    #[arg(long)]
    pub tags: Option<String>,
    /// Occupant identity recorded on the lease (default: session host/pid).
    #[arg(long)]
    pub occupant: Option<String>,
    /// Push live CC observer traffic to this session after reloading the bridge.
    #[arg(long)]
    pub wake_on_cc: bool,
}

#[derive(Args)]
pub struct CopilotSessionEndArgs {
    /// Stable Copilot session identity; defaults to hook stdin or COPILOT_AGENT_SESSION_ID.
    #[arg(long)]
    pub session: Option<String>,
}

#[derive(Args)]
pub struct CopilotTurnGuardArgs {
    /// Stable Copilot session identity; defaults to hook stdin or COPILOT_AGENT_SESSION_ID.
    #[arg(long)]
    pub session: Option<String>,
}

#[derive(Args)]
pub struct CopilotPushArgs {
    /// Stable Copilot session identity whose bridge should receive the message;
    /// defaults to COPILOT_AGENT_SESSION_ID.
    #[arg(long)]
    pub session: Option<String>,
}

#[derive(Args)]
pub struct CopilotDrainArgs {
    /// Stable Copilot session identity to drain; defaults to hook stdin or COPILOT_AGENT_SESSION_ID.
    #[arg(long)]
    pub session: Option<String>,
}

#[derive(Args)]
pub struct CopilotDetachArgs {
    /// Stable Copilot session identity; defaults to COPILOT_AGENT_SESSION_ID.
    #[arg(long)]
    pub session: Option<String>,
}

#[derive(Subcommand)]
pub enum CopilotFallbackCmd {
    /// Prepare one idempotent detached-waiter run and print its platform launcher.
    Prepare(CopilotFallbackPrepareArgs),
    /// Execute a prepared run. Invoked by the detached launcher, not directly by agents.
    #[command(hide = true)]
    Run(CopilotFallbackRunArgs),
}

#[derive(Args)]
pub struct CopilotFallbackPrepareArgs {
    /// Stable Copilot session identity; defaults to COPILOT_AGENT_SESSION_ID.
    #[arg(long)]
    pub session: Option<String>,
    /// One-line directory description used when the station must be attached.
    #[arg(long)]
    pub description: Option<String>,
    /// Project/workstream scope used when the station must be attached.
    #[arg(long)]
    pub scope: Option<String>,
    /// Comma-separated coarse tags used when the station must be attached.
    #[arg(long)]
    pub tags: Option<String>,
    /// Occupant identity used when the station must be attached.
    #[arg(long)]
    pub occupant: Option<String>,
    /// Give up after this many milliseconds and produce an idle-timeout artifact.
    #[arg(long, default_value_t = 1_800_000)]
    pub timeout_ms: u64,
    /// Only wake for messages at this attention or higher priority.
    #[arg(long, value_parser = parse_attention_arg)]
    pub min_attention: Option<Attention>,
    /// Also wake for live CC traffic without making CC ack-required.
    #[arg(long)]
    pub wake_on_cc: bool,
    /// Deliberately leave a currently live push bridge for pull fallback.
    #[arg(long)]
    pub force: bool,
}

#[derive(Args)]
pub struct CopilotFallbackRunArgs {
    /// Prepared fallback run directory containing fallback.json.
    #[arg(long)]
    pub run_dir: PathBuf,
}

#[derive(Args)]
pub struct CopilotGcArgs {
    /// Stable Copilot session identity to check/remove; defaults to all known bridge files.
    #[arg(long)]
    pub session: Option<String>,
    /// Report what would be removed without deleting anything.
    #[arg(long)]
    pub dry_run: bool,
    /// Remove stale files even when liveness is uncertain. Live registry heartbeats are still kept.
    #[arg(long)]
    pub force: bool,
}

#[derive(Args)]
pub struct CopilotSkillArgs {
    /// The invoking telex plugin's version, for a plugin/binary compatibility check.
    /// Falls back to the TELEX_PLUGIN_VERSION environment variable.
    #[arg(long)]
    pub plugin_version: Option<String>,
}

#[derive(Args)]
pub struct SkillArgs {
    /// Tailor the instructions for a specific assigned address.
    #[arg(long)]
    pub address: Option<String>,
    /// Print the embedded SKILL.md verbatim (including frontmatter).
    #[arg(long)]
    pub raw: bool,
}

#[derive(Subcommand)]
pub enum BackendCmd {
    /// Add (or update) a named backend.
    Add(BackendAddArgs),
    /// List configured backends.
    List,
    /// Show one backend's configuration (secrets redacted).
    Show {
        /// Backend name.
        name: String,
    },
    /// Remove a configured backend.
    Remove {
        /// Backend name.
        name: String,
    },
    /// Set the default backend.
    Default {
        /// Backend name.
        name: String,
    },
    /// List the backend kinds compiled into this build.
    Kinds,
}

#[derive(Args)]
pub struct BackendAddArgs {
    /// Name (key) for this backend.
    pub name: String,
    /// Configure a SQLite backend (path defaults to ~/.telex/telex.db).
    #[arg(long)]
    pub sqlite: bool,
    /// Configure a Postgres backend from this connection string (libpq URI or key=value DSN).
    #[arg(long, value_name = "CONN")]
    pub postgres: Option<String>,
    /// SQLite file path (with --sqlite).
    #[arg(long)]
    pub path: Option<String>,
    /// Postgres schema to isolate telex tables in.
    #[arg(long)]
    pub schema: Option<String>,
    /// Read the Postgres password from this environment variable.
    #[arg(long)]
    pub password_env: Option<String>,
    /// Obtain the Postgres password by running this shell command (its stdout).
    #[arg(long)]
    pub password_command: Option<String>,
    /// Use Microsoft Entra auth for Postgres (token fetched via the Azure SDK).
    #[arg(long)]
    pub entra: bool,
    /// Entra credential mode: auto (dev/CLI login), cli, or managed (devbox/VM identity).
    #[arg(long, value_name = "MODE")]
    pub entra_cred: Option<String>,
    /// Override the Entra token scope.
    #[arg(long)]
    pub entra_scope: Option<String>,
    /// Make this the default backend.
    #[arg(long)]
    pub default: bool,
}

fn parse_watch_pid(raw: &str) -> std::result::Result<WatchPidSpec, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("watch pid cannot be empty".to_string());
    }

    let parse_role = |s: &str| match s.to_ascii_lowercase().as_str() {
        "anchor" => Ok(WatchPidRole::Anchor),
        "required" => Ok(WatchPidRole::Required),
        other => Err(format!(
            "unknown watch-pid role {other:?}; use anchor or required"
        )),
    };
    let parse_pid = |s: &str| {
        s.parse::<u32>()
            .map_err(|e| format!("invalid watch pid {s:?}: {e}"))
    };

    if let Some((a, b)) = raw.split_once(':') {
        if let Ok(role) = parse_role(a) {
            return Ok(WatchPidSpec {
                pid: parse_pid(b)?,
                role,
            });
        }
        if let Ok(role) = parse_role(b) {
            return Ok(WatchPidSpec {
                pid: parse_pid(a)?,
                role,
            });
        }
        return Err(format!(
            "watch pid {raw:?} must be PID, role:PID, or PID:role"
        ));
    }

    Ok(WatchPidSpec::anchor(parse_pid(raw)?))
}

fn parse_attention_arg(raw: &str) -> std::result::Result<Attention, String> {
    Attention::parse(raw).map_err(|e| e.to_string())
}

/// Shared command context.
pub struct Ctx {
    pub cfg: Config,
    pub fmt: Format,
    pub address: Option<String>,
}

impl Ctx {
    /// Resolve and build the selected backend (initializing its schema).
    pub async fn backend(&self) -> Result<Arc<dyn Backend>> {
        let (_name, profile) = self.resolved()?;
        crate::profiles::build(&profile, self.cfg.db_override.as_deref()).await
    }

    /// Resolve which backend is selected, without connecting.
    pub fn resolved(&self) -> Result<(String, BackendProfile)> {
        crate::profiles::resolve(
            self.cfg.backend_selector.as_deref(),
            self.cfg.db_override.as_deref(),
        )
    }

    /// Effective store key for the selected backend, used to scope the holder registry so a
    /// station on one store is never inferred as `from` for a send on another.
    pub fn store_key(&self) -> Result<String> {
        let (_name, profile) = self.resolved()?;
        Ok(crate::profiles::store_key(
            &profile,
            self.cfg.db_override.as_deref(),
        ))
    }
}

pub async fn run() -> i32 {
    let cli = Cli::parse();
    let fmt = Format::resolve(cli.json, cli.text);
    let cfg = match Config::resolve(cli.backend, cli.db, cli.address.clone()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("telex: {e:#}");
            return 1;
        }
    };
    let ctx = Ctx {
        cfg,
        fmt,
        address: cli.address,
    };

    let result: Result<i32> = match cli.command {
        Command::Init => crate::commands::init::run(&ctx).await,
        Command::Status => crate::commands::status::run(&ctx).await,
        Command::Version(a) => crate::commands::upgrade::version(&ctx, a).await,
        Command::Upgrade(a) => crate::commands::upgrade::upgrade(&ctx, a).await,
        Command::Rollback(a) => crate::commands::upgrade::rollback(&ctx, a).await,
        Command::Gc(a) => crate::commands::upgrade::gc(&ctx, a).await,
        Command::Skill(a) => crate::commands::skill::run(&ctx, a).await,
        Command::Attach(a) => crate::commands::attach::run(&ctx, a).await,
        Command::Detach(a) => crate::commands::detach::run(&ctx, a).await,
        Command::Station(cmd) => crate::commands::station::run(&ctx, cmd).await,
        Command::Wait(a) => crate::commands::wait::run(&ctx, a).await,
        Command::Inbox(a) => crate::commands::inbox::run(&ctx, a).await,
        Command::Read(a) => crate::commands::read::run(&ctx, a).await,
        Command::Send(a) => crate::commands::send::run(&ctx, a).await,
        Command::Reply(a) => crate::commands::reply::run(&ctx, a).await,
        Command::Ack(a) => crate::commands::disposition::ack(&ctx, a).await,
        Command::Handle(a) => crate::commands::disposition::run(&ctx, "handled", a).await,
        Command::Defer(a) => crate::commands::disposition::run(&ctx, "deferred", a).await,
        Command::Reject(a) => crate::commands::disposition::run(&ctx, "rejected", a).await,
        Command::Close(a) => crate::commands::disposition::run(&ctx, "closed", a).await,
        Command::Escalate(a) => crate::commands::disposition::run(&ctx, "escalated", a).await,
        Command::Address(cmd) => crate::commands::address::run(&ctx, cmd).await,
        Command::Resolve(a) => crate::commands::address::resolve(&ctx, a).await,
        Command::Backend(cmd) => crate::commands::backend::run(&ctx, cmd).await,
        Command::Copilot(cmd) => crate::commands::copilot::run(&ctx, cmd).await,
        Command::Daemon(cmd) => crate::commands::daemon::run(&ctx, cmd).await,
        Command::Export(a) => crate::commands::export::run(&ctx, a).await,
    };

    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("telex: {e:#}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[test]
    fn daemon_subcommand_is_hidden_from_top_level_help() {
        let help = Cli::command().render_long_help().to_string();
        assert!(
            !help.contains("daemon"),
            "top-level help leaked daemon:\n{help}"
        );
        assert!(
            !help.contains("copilot"),
            "top-level help leaked copilot adapter:\n{help}"
        );
    }

    #[test]
    fn hidden_daemon_subcommands_parse() {
        assert!(matches!(
            Cli::try_parse_from(["telex", "daemon", "serve"])
                .unwrap()
                .command,
            Command::Daemon(DaemonCmd::Serve)
        ));
        assert!(matches!(
            Cli::try_parse_from(["telex", "daemon", "status"])
                .unwrap()
                .command,
            Command::Daemon(DaemonCmd::Status)
        ));
        assert!(matches!(
            Cli::try_parse_from(["telex", "daemon", "version"])
                .unwrap()
                .command,
            Command::Daemon(DaemonCmd::Version)
        ));
        assert!(matches!(
            Cli::try_parse_from(["telex", "daemon", "stop", "--drain"])
                .unwrap()
                .command,
            Command::Daemon(DaemonCmd::Stop(DaemonStopArgs { drain: true }))
        ));
    }

    #[test]
    fn hidden_copilot_subcommands_parse() {
        assert!(matches!(
            Cli::try_parse_from([
                "telex",
                "--address",
                "addr:a",
                "copilot",
                "attach",
                "--description",
                "work",
            ])
            .unwrap()
            .command,
            Command::Copilot(CopilotCmd::Attach(CopilotAttachArgs {
                description: Some(d),
                ..
            })) if d == "work"
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "telex",
                "--address",
                "addr:a",
                "copilot",
                "attach",
                "--copilot-bridge",
                "--wake-on-cc",
            ])
            .unwrap()
            .command,
            Command::Copilot(CopilotCmd::Attach(CopilotAttachArgs {
                copilot_bridge: true,
                wake_on_cc: true,
                ..
            }))
        ));
        assert!(matches!(
            Cli::try_parse_from(["telex", "copilot", "session-end"])
                .unwrap()
                .command,
            Command::Copilot(CopilotCmd::SessionEnd(_))
        ));
        assert!(matches!(
            Cli::try_parse_from(["telex", "copilot", "turn-guard"])
                .unwrap()
                .command,
            Command::Copilot(CopilotCmd::TurnGuard(_))
        ));
        assert!(matches!(
            Cli::try_parse_from(["telex", "copilot", "skill", "--plugin-version", "0.1.0"])
                .unwrap()
                .command,
            Command::Copilot(CopilotCmd::Skill(CopilotSkillArgs {
                plugin_version: Some(v),
            })) if v == "0.1.0"
        ));
        assert!(matches!(
            Cli::try_parse_from(["telex", "copilot", "gc", "--dry-run"])
                .unwrap()
                .command,
            Command::Copilot(CopilotCmd::Gc(CopilotGcArgs { dry_run: true, .. }))
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "telex",
                "--address",
                "addr:a",
                "copilot",
                "fallback",
                "prepare",
                "--timeout-ms",
                "5000",
                "--force",
            ])
            .unwrap()
            .command,
            Command::Copilot(CopilotCmd::Fallback(CopilotFallbackCmd::Prepare(
                CopilotFallbackPrepareArgs {
                    timeout_ms: 5000,
                    force: true,
                    ..
                }
            )))
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "telex",
                "copilot",
                "fallback",
                "run",
                "--run-dir",
                "/tmp/fallback-run",
            ])
            .unwrap()
            .command,
            Command::Copilot(CopilotCmd::Fallback(CopilotFallbackCmd::Run(
                CopilotFallbackRunArgs { .. }
            )))
        ));
    }

    #[test]
    fn upgrade_surface_subcommands_parse() {
        let cli = Cli::try_parse_from([
            "telex",
            "upgrade",
            "--from",
            "target/debug/telex",
            "--version",
            "v0.2.0",
            "--no-switch",
        ])
        .unwrap();
        match cli.command {
            Command::Upgrade(UpgradeArgs {
                from: Some(from),
                version: Some(version),
                no_switch: true,
                repo,
                ..
            }) => {
                assert_eq!(from, PathBuf::from("target/debug/telex"));
                assert_eq!(version, "v0.2.0");
                assert_eq!(repo, "lossyrob/telex");
            }
            _ => panic!("unexpected upgrade parse"),
        }

        // Release path: no --from is now valid and selects the GitHub release flow.
        let cli =
            Cli::try_parse_from(["telex", "upgrade", "--version", "v0.2.0", "--force"]).unwrap();
        match cli.command {
            Command::Upgrade(UpgradeArgs {
                from: None,
                version: Some(version),
                force: true,
                ..
            }) => assert_eq!(version, "v0.2.0"),
            _ => panic!("unexpected release-path upgrade parse"),
        }

        // Bare `telex upgrade` (latest release) parses with all defaults.
        let cli = Cli::try_parse_from(["telex", "upgrade"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Upgrade(UpgradeArgs { from: None, .. })
        ));

        let cli = Cli::try_parse_from(["telex", "rollback", "--version", "v0.1.0"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Rollback(RollbackArgs {
                version: Some(_),
                ..
            })
        ));

        let cli = Cli::try_parse_from(["telex", "gc", "--dry-run", "--force"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Gc(GcArgs {
                dry_run: true,
                force: true,
                ..
            })
        ));

        let cli = Cli::try_parse_from(["telex", "version"]).unwrap();
        assert!(matches!(cli.command, Command::Version(VersionArgs { .. })));
    }

    #[test]
    fn copilot_help_lists_user_facing_subcommands_and_hides_internal_ones() {
        let mut cmd = Cli::command();
        let copilot = cmd
            .find_subcommand_mut("copilot")
            .expect("copilot subcommand exists");
        let help = copilot.render_long_help().to_string();
        for sub in ["skill", "attach", "detach", "gc"] {
            assert!(
                help.contains(sub),
                "`telex copilot --help` should list `{sub}`:\n{help}"
            );
        }
        for hidden in ["session-end", "turn-guard"] {
            assert!(
                !help.contains(hidden),
                "`telex copilot --help` leaked internal `{hidden}`:\n{help}"
            );
        }
    }

    #[test]
    fn station_status_subcommand_parses() {
        assert!(matches!(
            Cli::try_parse_from(["telex", "station", "status", "--session", "s1"])
                .unwrap()
                .command,
            Command::Station(StationCmd::Status(StationStatusArgs {
                session: Some(s),
                all_sessions: false,
            })) if s == "s1"
        ));
        assert!(matches!(
            Cli::try_parse_from(["telex", "station", "status", "--all-sessions"])
                .unwrap()
                .command,
            Command::Station(StationCmd::Status(StationStatusArgs {
                all_sessions: true,
                ..
            }))
        ));
    }

    #[test]
    fn wait_reconnect_grace_flag_parses() {
        let cli = Cli::try_parse_from([
            "telex",
            "--address",
            "addr:a",
            "wait",
            "--reconnect-grace-ms",
            "250",
        ])
        .unwrap();
        let Command::Wait(args) = cli.command else {
            panic!("expected wait command");
        };
        assert_eq!(args.reconnect_grace_ms, Some(250));
    }

    #[test]
    fn wait_min_attention_flag_parses_and_validates() {
        let cli = Cli::try_parse_from([
            "telex",
            "--address",
            "addr:a",
            "wait",
            "--min-attention",
            "next-checkpoint",
        ])
        .unwrap();
        let Command::Wait(args) = cli.command else {
            panic!("expected wait command");
        };
        assert_eq!(args.min_attention, Some(Attention::NextCheckpoint));

        assert!(Cli::try_parse_from([
            "telex",
            "--address",
            "addr:a",
            "wait",
            "--min-attention",
            "urgent",
        ])
        .is_err());
    }

    #[test]
    fn wait_wake_on_cc_flag_parses() {
        let cli =
            Cli::try_parse_from(["telex", "--address", "addr:a", "wait", "--wake-on-cc"]).unwrap();
        let Command::Wait(args) = cli.command else {
            panic!("expected wait command");
        };
        assert!(args.wake_on_cc);
    }

    #[test]
    fn send_cc_accepts_repeated_and_comma_separated_values() {
        let cli = Cli::try_parse_from([
            "telex",
            "--address",
            "addr:sender",
            "send",
            "--to",
            "addr:to",
            "--body",
            "hello",
            "--cc",
            "addr:a,addr:b",
            "--cc",
            "addr:c",
        ])
        .unwrap();
        let Command::Send(args) = cli.command else {
            panic!("expected send command");
        };
        assert_eq!(args.cc, vec!["addr:a", "addr:b", "addr:c"]);
    }

    #[test]
    fn reply_cc_accepts_repeated_and_comma_separated_values() {
        let cli = Cli::try_parse_from([
            "telex",
            "--address",
            "addr:sender",
            "reply",
            "--to-message",
            "42",
            "--body",
            "hello",
            "--cc",
            "addr:a,addr:b",
            "--cc",
            "addr:c",
        ])
        .unwrap();
        let Command::Reply(args) = cli.command else {
            panic!("expected reply command");
        };
        assert_eq!(args.cc, vec!["addr:a", "addr:b", "addr:c"]);
    }

    #[test]
    fn attach_watch_pid_flag_accepts_typed_repeatable_values() {
        let cli = Cli::try_parse_from([
            "telex",
            "--address",
            "addr:a",
            "attach",
            "--watch-pid",
            "anchor:123",
            "--watch-pid",
            "456:required",
        ])
        .unwrap();
        let Command::Attach(args) = cli.command else {
            panic!("expected attach command");
        };
        assert_eq!(args.watch_pid.len(), 2);
        assert_eq!(args.watch_pid[0].pid, 123);
        assert_eq!(args.watch_pid[0].role, WatchPidRole::Anchor);
        assert_eq!(args.watch_pid[1].pid, 456);
        assert_eq!(args.watch_pid[1].role, WatchPidRole::Required);
    }
}
