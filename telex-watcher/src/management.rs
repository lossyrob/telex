//! In-process local administration. All mutations run through the registry owner.

use crate::evidence::{Attempt, Operation, ReceiptEvidence, RuntimeHealth, Watch, WatchHealth};
use crate::registry_v2::Mutation;
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type",
    content = "value",
    deny_unknown_fields
)]
pub enum Command {
    Register {
        json: Vec<u8>,
        working_directory: PathBuf,
    },
    List {
        include_removed: bool,
        include_unsupported: bool,
    },
    Show {
        id: String,
    },
    Pause {
        id: String,
    },
    Resume {
        id: String,
    },
    Update {
        id: String,
        json: Vec<u8>,
        working_directory: PathBuf,
        /// None preserves state; Some(Value::Null) replaces it with JSON null.
        #[serde(
            default,
            deserialize_with = "present_state",
            skip_serializing_if = "Option::is_none"
        )]
        state: Option<serde_json::Value>,
    },
    ReconcileNow {
        id: String,
    },
    GrantReconciliation {
        id: String,
        attempts: u32,
        seconds: u64,
    },
    GrantSendRetry {
        id: String,
        attempts: u32,
        seconds: u64,
    },
    CloseNotAccepted {
        id: String,
    },
    Remove {
        id: String,
    },
    Health,
    WatchHealth {
        id: String,
    },
    Attempts {
        id: String,
        limit: u32,
    },
    Events {
        id: String,
        limit: u32,
    },
    Pending {
        id: String,
    },
    ReceiptEvidence {
        operation_id: String,
        limit: u32,
    },
    RemovedEvidence {
        id: String,
        limit: u32,
    },
    Shutdown,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(
    rename_all = "camelCase",
    tag = "type",
    content = "value",
    deny_unknown_fields
)]
pub enum Response {
    Mutation(Mutation),
    Watch(Option<Watch>),
    Watches(Vec<Watch>),
    Health(RuntimeHealth),
    WatchHealth(WatchHealth),
    Attempts(Vec<Attempt>),
    Events(Vec<Operation>),
    Pending(Option<Box<Operation>>),
    ReceiptEvidence(Vec<ReceiptEvidence>),
    Stopping,
}

fn present_state<'de, D>(deserializer: D) -> Result<Option<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(deserializer).map(Some)
}

pub(crate) struct Request {
    pub command: Command,
    pub reply: oneshot::Sender<anyhow::Result<Response>>,
}

/// Cloneable, bounded local handle. Dropping a caller does not roll back an
/// admitted mutation; inspect the registry before retrying it.
#[derive(Clone)]
pub struct LocalManagement {
    sender: mpsc::Sender<Request>,
}

impl LocalManagement {
    pub(crate) fn channel() -> (Self, mpsc::Receiver<Request>) {
        let (sender, receiver) = mpsc::channel(64);
        (Self { sender }, receiver)
    }

    pub async fn execute(&self, command: Command) -> anyhow::Result<Response> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(Request { command, reply })
            .await
            .map_err(|_| anyhow::anyhow!("watcher-owner-unavailable"))?;
        receive
            .await
            .map_err(|_| anyhow::anyhow!("watcher-owner-outcome-unknown"))?
    }
}
