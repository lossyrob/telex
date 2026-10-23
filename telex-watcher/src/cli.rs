//! Thin local management parser. Validation and mutation authority remain in the owner.

use crate::evidence::{BlockedReason, Lifecycle, Watch};
use crate::management::Command;
use crate::protocol_v2;
use clap::{Args, Parser, Subcommand};
use std::path::{Path, PathBuf};

const MAX_REGISTRATION_FILE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Parser)]
#[command(version, about = "Telex Watcher v2 local management and runtime host")]
pub struct Cli {
    #[arg(long, env = "TELEX_WATCHER_REGISTRY", global = true)]
    pub registry: Option<PathBuf>,
    #[arg(long, env = "TELEX_INSTALL_ROOT", global = true)]
    pub install_root: Option<PathBuf>,
    #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=16), global = true)]
    pub concurrency: u8,
    /// Emit the versioned, safe management envelope.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub action: Option<Action>,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Start the registry owner (also the default when no command is given).
    Run,
    /// Register a trusted-local v2 registration file.
    Register {
        file: PathBuf,
    },
    /// List registrations; lifecycle selectors are combined as a union.
    List {
        #[command(flatten)]
        filters: Filters,
    },
    /// Inspect a fixed watch ID, including its effective policy.
    Show {
        id: String,
        #[command(flatten)]
        filters: Filters,
    },
    Pause {
        id: String,
    },
    Resume {
        id: String,
    },
    /// Replace mutable registration fields; omitted state preserves committed state.
    Update {
        id: String,
        file: PathBuf,
        /// Strict JSON committed-state replacement. A file containing null commits null.
        #[arg(long)]
        state_file: Option<PathBuf>,
    },
    /// Query the exact staged operation without authorizing a send.
    ReconcileNow {
        id: String,
    },
    /// Grant a finite query-only recovery budget.
    GrantReconciliation {
        id: String,
        #[command(flatten)]
        grant: Grant,
    },
    /// Grant finite retries of the exact staged send, subject to owner proof.
    GrantSendRetry {
        id: String,
        #[command(flatten)]
        grant: Grant,
    },
    CloseNotAccepted {
        id: String,
    },
    Remove {
        id: String,
    },
    /// Runtime status, or per-watch health when an ID is supplied.
    Status {
        id: Option<String>,
    },
    /// Runtime health, or per-watch health when an ID is supplied.
    Health {
        id: Option<String>,
    },
    Attempts {
        id: String,
        #[command(flatten)]
        query: Query,
    },
    Events {
        id: String,
        #[command(flatten)]
        query: Query,
    },
    Pending {
        id: String,
    },
    ReceiptEvidence {
        operation_id: String,
        #[command(flatten)]
        query: Query,
    },
    RemovedEvidence {
        id: String,
        #[command(flatten)]
        query: Query,
    },
    /// Stop the owner, including a runtime started as a detached process.
    #[command(alias = "stop")]
    Shutdown,
}

#[derive(Debug, Args)]
pub struct Grant {
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=100))]
    pub attempts: u32,
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..=86_400))]
    pub seconds: u64,
}

#[derive(Debug, Args)]
pub struct Query {
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=1000))]
    pub limit: u32,
}

#[derive(Debug, Default, Clone, Args, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Filters {
    #[arg(long)]
    pub active: bool,
    #[arg(long)]
    pub paused: bool,
    #[arg(long)]
    pub terminal: bool,
    #[arg(long)]
    pub removed: bool,
    #[arg(long)]
    pub unsupported_v1: bool,
    #[arg(long)]
    pub unresolved_tombstones: bool,
    /// Include removed records without restricting other lifecycle states.
    #[arg(long)]
    pub include_removed: bool,
    /// Include unsupported records without restricting other lifecycle states.
    #[arg(long, alias = "include-unsupported-v1")]
    pub include_unsupported: bool,
    #[arg(long)]
    pub all: bool,
}

impl Filters {
    pub fn is_explicit(&self) -> bool {
        self.active
            || self.paused
            || self.terminal
            || self.removed
            || self.unsupported_v1
            || self.unresolved_tombstones
            || self.include_removed
            || self.include_unsupported
            || self.all
    }

    pub fn include_removed(&self) -> bool {
        self.all || self.include_removed || self.removed || self.unresolved_tombstones
    }

    pub fn include_unsupported(&self) -> bool {
        self.all || self.include_unsupported || self.unsupported_v1
    }

    pub fn matches(&self, watch: &Watch) -> bool {
        let unsupported = watch.schema_version != 2
            || watch.blocked_reason == Some(BlockedReason::UnsupportedRegistrationVersion);
        if self.all {
            return true;
        }
        if unsupported && !self.include_unsupported() {
            return false;
        }
        if watch.lifecycle == Lifecycle::Removed && !self.include_removed() {
            return false;
        }
        let selected = self.active
            || self.paused
            || self.terminal
            || self.removed
            || self.unsupported_v1
            || self.unresolved_tombstones;
        !selected
            || (self.active && watch.lifecycle == Lifecycle::Active)
            || (self.paused && watch.lifecycle == Lifecycle::Paused)
            || (self.terminal && watch.lifecycle == Lifecycle::Terminal)
            || (self.removed && watch.lifecycle == Lifecycle::Removed)
            || (self.unsupported_v1 && unsupported)
            || (self.unresolved_tombstones
                && watch.lifecycle == Lifecycle::Removed
                && watch.pending_operation_id.is_some())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InputError {
    pub code: &'static str,
    pub message: &'static str,
}

impl Cli {
    pub fn is_runtime(&self) -> bool {
        matches!(self.action, None | Some(Action::Run))
    }

    pub fn command_name(&self) -> &'static str {
        match &self.action {
            None | Some(Action::Run) => "run",
            Some(Action::Register { .. }) => "register",
            Some(Action::List { .. }) => "list",
            Some(Action::Show { .. }) => "show",
            Some(Action::Pause { .. }) => "pause",
            Some(Action::Resume { .. }) => "resume",
            Some(Action::Update { .. }) => "update",
            Some(Action::ReconcileNow { .. }) => "reconcile-now",
            Some(Action::GrantReconciliation { .. }) => "grant-reconciliation",
            Some(Action::GrantSendRetry { .. }) => "grant-send-retry",
            Some(Action::CloseNotAccepted { .. }) => "close-not-accepted",
            Some(Action::Remove { .. }) => "remove",
            Some(Action::Status { .. }) => "status",
            Some(Action::Health { .. }) => "health",
            Some(Action::Attempts { .. }) => "attempts",
            Some(Action::Events { .. }) => "events",
            Some(Action::Pending { .. }) => "pending",
            Some(Action::ReceiptEvidence { .. }) => "receipt-evidence",
            Some(Action::RemovedEvidence { .. }) => "removed-evidence",
            Some(Action::Shutdown) => "shutdown",
        }
    }

    pub fn filters(&self) -> Option<&Filters> {
        match &self.action {
            Some(Action::List { filters }) => Some(filters),
            Some(Action::Show { filters, .. }) if filters.is_explicit() => Some(filters),
            _ => None,
        }
    }

    pub fn management_command(&self) -> Result<Command, InputError> {
        let id_command = |id: &String| id.clone();
        Ok(match self.action.as_ref() {
            None | Some(Action::Run) => {
                return Err(InputError {
                    code: "invalid-command",
                    message: "run starts the runtime host, not a management request",
                })
            }
            Some(Action::Register { file }) => Command::Register {
                json: read_file(file, MAX_REGISTRATION_FILE_BYTES, "registration-file")?,
                working_directory: current_directory()?,
            },
            Some(Action::List { filters }) => Command::List {
                include_removed: filters.include_removed(),
                include_unsupported: filters.include_unsupported(),
            },
            Some(Action::Show { id, .. }) => Command::Show { id: id_command(id) },
            Some(Action::Pause { id }) => Command::Pause { id: id_command(id) },
            Some(Action::Resume { id }) => Command::Resume { id: id_command(id) },
            Some(Action::Update {
                id,
                file,
                state_file,
            }) => Command::Update {
                id: id_command(id),
                json: read_file(file, MAX_REGISTRATION_FILE_BYTES, "registration-file")?,
                working_directory: current_directory()?,
                state: state_file.as_deref().map(read_state).transpose()?,
            },
            Some(Action::ReconcileNow { id }) => Command::ReconcileNow { id: id_command(id) },
            Some(Action::GrantReconciliation { id, grant }) => Command::GrantReconciliation {
                id: id_command(id),
                attempts: grant.attempts,
                seconds: grant.seconds,
            },
            Some(Action::GrantSendRetry { id, grant }) => Command::GrantSendRetry {
                id: id_command(id),
                attempts: grant.attempts,
                seconds: grant.seconds,
            },
            Some(Action::CloseNotAccepted { id }) => {
                Command::CloseNotAccepted { id: id_command(id) }
            }
            Some(Action::Remove { id }) => Command::Remove { id: id_command(id) },
            Some(Action::Status { id } | Action::Health { id }) => match id {
                Some(id) => Command::WatchHealth { id: id_command(id) },
                None => Command::Health,
            },
            Some(Action::Attempts { id, query }) => Command::Attempts {
                id: id_command(id),
                limit: query.limit,
            },
            Some(Action::Events { id, query }) => Command::Events {
                id: id_command(id),
                limit: query.limit,
            },
            Some(Action::Pending { id }) => Command::Pending { id: id_command(id) },
            Some(Action::ReceiptEvidence {
                operation_id,
                query,
            }) => Command::ReceiptEvidence {
                operation_id: operation_id.clone(),
                limit: query.limit,
            },
            Some(Action::RemovedEvidence { id, query }) => Command::RemovedEvidence {
                id: id_command(id),
                limit: query.limit,
            },
            Some(Action::Shutdown) => Command::Shutdown,
        })
    }
}

fn current_directory() -> Result<PathBuf, InputError> {
    std::env::current_dir().map_err(|_| InputError {
        code: "working-directory-unavailable",
        message: "cannot capture the current absolute working directory",
    })
}

fn read_file(path: &Path, cap: usize, kind: &'static str) -> Result<Vec<u8>, InputError> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|_| InputError {
        code: kind,
        message: "cannot open the local input file",
    })?;
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| InputError {
            code: kind,
            message: "cannot read the local input file",
        })?;
    if bytes.len() > cap {
        return Err(InputError {
            code: "input-too-large",
            message: "local input exceeds the byte limit",
        });
    }
    Ok(bytes)
}

fn read_state(path: &Path) -> Result<serde_json::Value, InputError> {
    let bytes = read_file(path, protocol_v2::MAX_STATE_BYTES, "state-file")?;
    let invalid = || InputError {
        code: "invalid-state",
        message: "state must satisfy strict v2 JSON, canonicalization and byte limits",
    };
    let state = protocol_v2::parse_json(&bytes).map_err(|_| invalid())?;
    let canonical = protocol_v2::canonical_bytes(&state).map_err(|_| invalid())?;
    if canonical.len() > protocol_v2::MAX_STATE_BYTES {
        return Err(invalid());
    }
    Ok(state)
}
