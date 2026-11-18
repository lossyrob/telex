//! The seam between Station and Telex.
//!
//! Two implementations exist:
//!
//! * [`crate::port_client`] uses the supported `telex::application_client`. This
//!   is the intended production path, and what Linux and Windows builds use.
//! * [`crate::port_cli`] drives the `telex` CLI as an ordinary session. It
//!   exists because, on macOS today, Telex deliberately refuses the Application
//!   Client daemon bootstrap ("running peer executable file identity is
//!   unsupported on this platform"), and the legacy path requires the client and
//!   daemon to be the same executable. The CLI *is* that executable, so a Mac
//!   prototype can attend an address honestly without working around the guard.
//!
//! Everything above this trait (projection, policy, organization, UI) is
//! identical for both.

use crate::model::{HealthView, OutboxItem, Presence, Receipt, StoredMessage};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct PortError {
    pub message: String,
    /// The operation may have been accepted; reconcile before retrying.
    pub uncertain: bool,
    /// Membership was lost; attach again before continuing.
    pub needs_attach: bool,
}

impl PortError {
    pub fn new(message: impl Into<String>) -> Self {
        PortError { message: message.into(), uncertain: false, needs_attach: false }
    }
    pub fn uncertain(message: impl Into<String>) -> Self {
        PortError { message: message.into(), uncertain: true, needs_attach: false }
    }
    pub fn needs_attach(message: impl Into<String>) -> Self {
        PortError { message: message.into(), uncertain: false, needs_attach: true }
    }
}

impl std::fmt::Display for PortError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PortError {}

#[derive(Clone, Debug)]
pub struct Sent {
    pub message_id: i64,
    pub thread_id: i64,
    pub sender: String,
    pub recipient: String,
    pub occupied: Option<bool>,
    pub replayed: bool,
}

#[derive(Clone, Debug)]
pub enum Reconciled {
    Accepted(Sent),
    /// Authoritatively absent: safe to retry or discard.
    NotRecorded,
    Unknown(String),
}

#[async_trait]
pub trait TelexPort: Send + Sync {
    /// "application-client" | "cli"
    fn kind(&self) -> &'static str;
    /// A limitation worth showing in the health panel, if any.
    fn note(&self) -> Option<String>;
    /// Opaque store identity for display. Never a path or credential.
    fn store_id(&self) -> String;

    async fn attach(&self, address: &str, description: &str) -> Result<(), PortError>;
    async fn detach(&self, address: &str) -> Result<(), PortError>;

    /// One exact delivery, not yet acknowledged.
    async fn receive(&self, address: &str, timeout_ms: u64) -> Result<Option<StoredMessage>, PortError>;
    /// Called only after the delivery is durable in the local projection.
    async fn acknowledge(&self, message: &StoredMessage) -> Result<(), PortError>;
    /// Messages for this address with id greater than `after_id`, plus the new cursor.
    async fn history(&self, address: &str, after_id: i64) -> Result<(Vec<StoredMessage>, i64), PortError>;

    /// Stage restart-safe recovery evidence before the first send attempt.
    async fn prepare(&self, item: &OutboxItem) -> Result<Option<Value>, PortError>;
    async fn submit(&self, item: &OutboxItem) -> Result<Sent, PortError>;
    async fn reconcile(&self, item: &OutboxItem) -> Result<Reconciled, PortError>;

    /// `Ok(false)` when this port cannot record that state.
    async fn disposition(
        &self,
        message: &StoredMessage,
        state: &str,
        note: Option<&str>,
    ) -> Result<bool, PortError>;
    /// Current receipt evidence for a message this Station authored.
    async fn receipt(&self, message: &StoredMessage) -> Result<Receipt, PortError>;

    async fn health(&self) -> Result<Vec<HealthView>, PortError>;
    async fn presence(&self) -> Result<BTreeMap<String, Presence>, PortError>;
}

pub type Port = Arc<dyn TelexPort>;
