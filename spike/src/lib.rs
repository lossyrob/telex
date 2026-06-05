//! Shared helpers for the Telex spike: Postgres connection (Entra token or SQL
//! password via env), schema, and the local IPC frame protocol between the
//! resident holder and the ephemeral waiter.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub mod backend;
pub use backend::{make_backend, Backend, MsgRow as BackendMsgRow, Occupancy};

pub fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

/// Single channel both the holder (push mode) and bench LISTEN on. The payload
/// carries the address + id so a listener can filter to its own address.
pub const NOTIFY_CHANNEL: &str = "telex_messages";

#[derive(Serialize, Deserialize, Debug)]
pub struct NotifyPayload {
    pub address: String,
    pub id: i64,
    pub sent_at_ms: i64,
}

/// Build the Postgres config. Password comes from `TELEX_PG_PASSWORD`, which may
/// be either an Entra access token or a SQL password — the spike treats them the
/// same so we can compare both auth paths without code changes.
pub fn pg_config() -> Result<tokio_postgres::Config> {
    let host = env_or("TELEX_PG_HOST", "pg-rde-telex.postgres.database.azure.com");
    let user = env_or("TELEX_PG_USER", "robemanuele@microsoft.com");
    let db = env_or("TELEX_PG_DB", "postgres");
    let password = std::env::var("TELEX_PG_PASSWORD")
        .context("TELEX_PG_PASSWORD must be set (Entra token or SQL password)")?;

    let mut config = tokio_postgres::Config::new();
    config
        .host(&host)
        .port(5432)
        .user(&user)
        .dbname(&db)
        .password(password)
        .ssl_mode(tokio_postgres::config::SslMode::Require);
    Ok(config)
}

pub fn make_tls() -> Result<postgres_native_tls::MakeTlsConnector> {
    let tls = native_tls::TlsConnector::builder()
        .build()
        .context("building TLS connector")?;
    Ok(postgres_native_tls::MakeTlsConnector::new(tls))
}

/// Connect and spawn the connection driver. Use for clients that only issue
/// queries (not the LISTEN connection, which must be driven via poll_message).
pub async fn connect() -> Result<tokio_postgres::Client> {
    let (client, connection) = pg_config()?
        .connect(make_tls()?)
        .await
        .context("connecting to postgres")?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("[db] connection task ended: {e}");
        }
    });
    Ok(client)
}

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS addresses (
    address     text PRIMARY KEY,
    description text,
    status      text NOT NULL DEFAULT 'active',
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS leases (
    address      text PRIMARY KEY,
    occupant     text,
    host         text,
    principal    text,
    heartbeat_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS messages (
    id         bigserial PRIMARY KEY,
    address    text NOT NULL,
    body       text NOT NULL,
    attention  text NOT NULL DEFAULT 'background',
    created_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE messages ADD COLUMN IF NOT EXISTS sent_at_ms bigint;

CREATE INDEX IF NOT EXISTS messages_address_id_idx ON messages (address, id);
"#;

/// Request sent by the waiter to the holder over local IPC.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Block until an actionable message is available (or the holder times out).
    Wait {
        address: String,
        #[serde(default)]
        since: i64,
        timeout_ms: u64,
    },
    /// Liveness probe; the holder answers immediately.
    Ping,
}

/// Frames the holder writes to the waiter over local IPC. One frame per line.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Frame {
    /// Periodic "I'm alive" signal carrying the age of the holder's last
    /// successful DB heartbeat, so the waiter can detect a degraded holder.
    Keepalive { heartbeat_age_ms: i64 },
    Pong { heartbeat_age_ms: i64 },
    /// Delivery — the waiter prints this and exits.
    Message {
        id: i64,
        address: String,
        body: String,
        attention: String,
        /// Client wall-clock ms when the sender inserted the message.
        sent_at_ms: i64,
        /// Holder wall-clock ms when it buffered the message.
        buffered_at_ms: i64,
    },
    /// Holder hit the waiter's requested idle timeout with no message.
    Timeout,
}
