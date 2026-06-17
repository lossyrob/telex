//! `telex-console` — a read-only, live-tailing terminal UI for inspecting a Telex
//! message fabric. It opens the same backend the `telex` CLI would (via the core
//! library's profile resolution) and reads messages, addresses, threads, and
//! dispositions. It never holds a lease, heartbeats, or mutates state.

mod app;
mod data;
mod event;
mod filter;
mod terminal;
mod ui;

use anyhow::Result;
use clap::Parser;

use crate::app::{AppState, Backfill};
use crate::data::Store;

/// Command-line surface. Mirrors the `telex` CLI's backend selection globals so the
/// console opens the same store.
#[derive(Parser, Debug)]
#[command(
    name = "telex-console",
    version,
    about = "Read-only, live-tailing TUI for inspecting Telex messages"
)]
struct Args {
    /// Configured backend to use, by name (default: the configured default backend).
    #[arg(long, env = "TELEX_BACKEND")]
    backend: Option<String>,

    /// Override the SQLite path for this invocation (sqlite backends only).
    #[arg(long, env = "TELEX_DB")]
    db: Option<String>,

    /// Address to focus by default.
    #[arg(long, env = "TELEX_ADDRESS")]
    address: Option<String>,

    /// Feed poll interval, in seconds.
    #[arg(long, default_value_t = 1)]
    poll_secs: u64,

    /// Recent messages to backfill on startup: a number, `0` (tail-only), or `all`.
    #[arg(long, default_value = "200")]
    backfill: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let backfill = Backfill::parse(&args.backfill)?;
    let (name, backend) = open_backend(&args).await?;
    let store = Store::new(backend);
    let kind = store.kind().to_string();

    let state = AppState::new(name, kind, args.address.clone(), backfill);
    app::run(state, store, args.poll_secs).await
}

/// Resolve and open the backend to inspect, reusing the core library's profile logic.
///
/// Selection precedence tuned for an inspector:
/// 1. `--backend <name>` → that configured profile (honoring `--db` for sqlite profiles).
/// 2. `--db <path>` alone → inspect that SQLite file directly (an implicit sqlite
///    backend), even when a non-sqlite default is configured. This is what a user means
///    by "point the console at this database file."
/// 3. neither → the configured default backend (or the zero-config implicit sqlite).
async fn open_backend(args: &Args) -> Result<(String, std::sync::Arc<dyn telex::backend::Backend>)> {
    let (name, profile) = if args.backend.is_some() {
        telex::profiles::resolve(args.backend.as_deref(), args.db.as_deref())?
    } else if args.db.is_some() {
        (
            "default".to_string(),
            telex::profiles::implicit_sqlite(args.db.as_deref()),
        )
    } else {
        telex::profiles::resolve(None, None)?
    };
    let backend = telex::profiles::build(&profile, args.db.as_deref()).await?;
    Ok((name, backend))
}
