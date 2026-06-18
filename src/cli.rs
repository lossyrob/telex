//! Command-line surface (clap) and dispatch. Global options resolve backend, db path,
//! default address, and output format; each subcommand maps to a handler in `commands`.

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use std::sync::Arc;

use crate::backend::Backend;
use crate::config::Config;
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
    /// Print the agent usage skill (how to use telex) for this build.
    Skill(SkillArgs),

    /// Start a station on an address: become the live occupant, hold the lease, run the holder (blocks).
    Attach(AttachArgs),
    /// Stop the station for an address: release the lease and stop a running holder.
    Detach,

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
    /// Lease heartbeat interval (seconds).
    #[arg(long, default_value_t = 5)]
    pub heartbeat_secs: u64,
    /// Backend poll interval (seconds).
    #[arg(long, default_value_t = 1)]
    pub poll_secs: u64,
    /// Keepalive frame interval to waiters (seconds).
    #[arg(long, default_value_t = 3)]
    pub keepalive_secs: u64,
    /// Occupant identity recorded on the lease (default: session host/pid).
    #[arg(long)]
    pub occupant: Option<String>,
    /// Enable Postgres LISTEN/NOTIFY push in addition to poll (no-op on SQLite).
    #[arg(long)]
    pub push: bool,
    /// Bind the holder's lifetime to this session/launcher pid: when that process exits, the
    /// holder releases its lease and exits (the same shutdown tail as `detach`/ctrl-c). The
    /// belt-and-suspenders companion to launching the holder background + session-bound — even a
    /// mis-launched detached holder cannot then outlive its session. Defaults from
    /// `$TELEX_SESSION_PID`.
    #[arg(long, env = "TELEX_SESSION_PID")]
    pub session_pid: Option<u32>,
    /// Interval (seconds) for the `--session-pid` liveness check; keep it well inside the lease
    /// liveness window so the address frees promptly.
    #[arg(long, default_value_t = 2)]
    pub session_poll_secs: u64,
    /// Do not bind to any session pid, even if `$TELEX_SESSION_PID` is set — for a deliberately
    /// persistent, server-side holder that should outlive its launcher. Overrides `--session-pid`.
    #[arg(long)]
    pub no_session_bind: bool,
}

#[derive(Args)]
pub struct WaitArgs {
    /// Give up waiting after this many milliseconds (exit code 2); default is no idle timeout.
    #[arg(long)]
    pub timeout_ms: Option<u64>,
    /// Resume delivery strictly after this message id.
    #[arg(long, default_value_t = 0)]
    pub since: i64,
    /// Treat the holder as hung if no frame arrives within this window (ms).
    #[arg(long, default_value_t = 8_000)]
    pub hang_ms: u64,
    /// Holder DB-heartbeat age beyond which it is considered degraded (ms).
    #[arg(long, default_value_t = 15_000)]
    pub stale_heartbeat_ms: i64,
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
    /// Message body (inline). Mutually exclusive with --body-file; exactly one is required.
    #[arg(long)]
    pub body: Option<String>,
    /// Read the message body from a UTF-8 file (`-` reads stdin). Mutually exclusive with --body.
    #[arg(long)]
    pub body_file: Option<String>,
    /// Comma-separated cc addresses (visible, not interrupting).
    #[arg(long)]
    pub cc: Option<String>,
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
    /// Arbitrary JSON metadata.
    #[arg(long)]
    pub metadata: Option<String>,
}

#[derive(Args)]
pub struct ReplyArgs {
    /// The message id being replied to.
    #[arg(long)]
    pub to_message: i64,
    /// Reply body (inline). Mutually exclusive with --body-file; exactly one is required.
    #[arg(long)]
    pub body: Option<String>,
    /// Read the reply body from a UTF-8 file (`-` reads stdin). Mutually exclusive with --body.
    #[arg(long)]
    pub body_file: Option<String>,
    /// Subject (defaults to "Re: <parent subject>").
    #[arg(long)]
    pub subject: Option<String>,
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
        Command::Skill(a) => crate::commands::skill::run(&ctx, a).await,
        Command::Attach(a) => crate::commands::attach::run(&ctx, a).await,
        Command::Detach => crate::commands::detach::run(&ctx).await,
        Command::Wait(a) => crate::commands::wait::run(&ctx, a).await,
        Command::Inbox(a) => crate::commands::inbox::run(&ctx, a).await,
        Command::Read(a) => crate::commands::read::run(&ctx, a).await,
        Command::Send(a) => crate::commands::send::run(&ctx, a).await,
        Command::Reply(a) => crate::commands::reply::run(&ctx, a).await,
        Command::Ack(a) => crate::commands::disposition::run(&ctx, "acknowledged", a).await,
        Command::Handle(a) => crate::commands::disposition::run(&ctx, "handled", a).await,
        Command::Defer(a) => crate::commands::disposition::run(&ctx, "deferred", a).await,
        Command::Reject(a) => crate::commands::disposition::run(&ctx, "rejected", a).await,
        Command::Close(a) => crate::commands::disposition::run(&ctx, "closed", a).await,
        Command::Escalate(a) => crate::commands::disposition::run(&ctx, "escalated", a).await,
        Command::Address(cmd) => crate::commands::address::run(&ctx, cmd).await,
        Command::Resolve(a) => crate::commands::address::resolve(&ctx, a).await,
        Command::Backend(cmd) => crate::commands::backend::run(&ctx, cmd).await,
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
