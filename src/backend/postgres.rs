//! Postgres backend: the networked substrate. Same semantic model as SQLite, with
//! epoch-ms integer timestamps for parity. v0 uses TTL-heartbeat liveness and
//! poll-with-cursor delivery; LISTEN/NOTIFY push is a best-effort extra. Auth via the
//! `TELEX_PG_*` env vars, where the password may be an Entra access token or a SQL password.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use tokio_postgres::Row;

use super::{Backend, Capabilities};
use crate::model::*;

pub const NOTIFY_CHANNEL: &str = "telex_messages";

pub struct PgBackend {
    client: tokio_postgres::Client,
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

pub fn pg_config() -> Result<tokio_postgres::Config> {
    let host = env_or("TELEX_PG_HOST", "localhost");
    let user = env_or("TELEX_PG_USER", "postgres");
    let db = env_or("TELEX_PG_DB", "postgres");
    let port: u16 = env_or("TELEX_PG_PORT", "5432").parse().unwrap_or(5432);
    let password = std::env::var("TELEX_PG_PASSWORD")
        .context("TELEX_PG_PASSWORD must be set (Entra access token or SQL password)")?;

    let mut config = tokio_postgres::Config::new();
    config
        .host(&host)
        .port(port)
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

pub async fn connect() -> Result<tokio_postgres::Client> {
    let (client, connection) = pg_config()?
        .connect(make_tls()?)
        .await
        .context("connecting to postgres")?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("[telex] postgres connection ended: {e}");
        }
    });
    // Optional schema isolation: keep telex tables in their own namespace so they can
    // coexist with other applications (or older spikes) in a shared database.
    if let Ok(schema) = std::env::var("TELEX_PG_SCHEMA") {
        let schema = sanitize_ident(&schema)?;
        client
            .batch_execute(&format!(
                "CREATE SCHEMA IF NOT EXISTS {schema}; SET search_path TO {schema}, public;"
            ))
            .await
            .context("setting TELEX_PG_SCHEMA search_path")?;
    }
    Ok(client)
}

/// Allow only a safe SQL identifier for the schema name (no injection via search_path).
fn sanitize_ident(s: &str) -> Result<String> {
    if !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && s.chars()
            .next()
            .map(|c| !c.is_ascii_digit())
            .unwrap_or(false)
    {
        Ok(s.to_string())
    } else {
        anyhow::bail!(
            "invalid TELEX_PG_SCHEMA '{s}' (use letters, digits, underscore; not leading digit)"
        )
    }
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS addresses (
    address       text PRIMARY KEY,
    description   text,
    scope         text,
    tags          text,
    status        text NOT NULL DEFAULT 'active',
    created_at_ms bigint NOT NULL
);
CREATE TABLE IF NOT EXISTS leases (
    address         text PRIMARY KEY,
    occupant        text,
    host            text,
    principal       text,
    description     text,
    tags            text,
    scope           text,
    pid             bigint,
    since_ms        bigint NOT NULL,
    heartbeat_at_ms bigint NOT NULL
);
CREATE TABLE IF NOT EXISTS messages (
    id            bigserial PRIMARY KEY,
    thread_id     bigint,
    parent_id     bigint,
    from_addr     text,
    to_addr       text NOT NULL,
    cc            text,
    kind          text NOT NULL DEFAULT 'note',
    attention     text NOT NULL DEFAULT 'background',
    requires_disposition boolean NOT NULL DEFAULT false,
    subject       text,
    body          text NOT NULL,
    metadata      text,
    sent_at_ms    bigint NOT NULL,
    created_at_ms bigint NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_to_id_idx ON messages(to_addr, id);
CREATE INDEX IF NOT EXISTS messages_thread_idx ON messages(thread_id, id);
CREATE TABLE IF NOT EXISTS dispositions (
    id           bigserial PRIMARY KEY,
    message_id   bigint NOT NULL,
    recipient    text NOT NULL,
    state        text NOT NULL,
    note         text,
    by_principal text,
    at_ms        bigint NOT NULL
);
CREATE INDEX IF NOT EXISTS dispositions_msg_idx ON dispositions(message_id, id);
"#;

const MSG_COLS: &str = "id, thread_id, parent_id, from_addr, to_addr, cc, kind, attention, \
    requires_disposition, subject, body, metadata, sent_at_ms, created_at_ms";

fn map_message(r: &Row) -> MessageRow {
    let id: i64 = r.get("id");
    let thread_id: Option<i64> = r.get("thread_id");
    MessageRow {
        id,
        thread_id: thread_id.unwrap_or(id),
        parent_id: r.get("parent_id"),
        from_addr: r.get("from_addr"),
        to_addr: r.get("to_addr"),
        cc: r.get("cc"),
        kind: r.get("kind"),
        attention: r.get("attention"),
        requires_disposition: r.get("requires_disposition"),
        subject: r.get("subject"),
        body: r.get("body"),
        metadata: r.get("metadata"),
        sent_at_ms: r.get("sent_at_ms"),
        created_at_ms: r.get("created_at_ms"),
    }
}

fn map_address(r: &Row) -> AddressRow {
    AddressRow {
        address: r.get("address"),
        description: r.get("description"),
        scope: r.get("scope"),
        tags: r.get("tags"),
        status: r.get("status"),
        created_at_ms: r.get("created_at_ms"),
    }
}

fn map_lease(r: &Row) -> LeaseRow {
    LeaseRow {
        address: r.get("address"),
        occupant: r.get("occupant"),
        host: r.get("host"),
        principal: r.get("principal"),
        description: r.get("description"),
        tags: r.get("tags"),
        scope: r.get("scope"),
        pid: r.get("pid"),
        since_ms: r.get("since_ms"),
        heartbeat_at_ms: r.get("heartbeat_at_ms"),
    }
}

impl PgBackend {
    pub async fn connect() -> Result<Self> {
        Ok(Self {
            client: connect().await?,
        })
    }
}

#[async_trait]
impl Backend for PgBackend {
    fn kind(&self) -> &'static str {
        "postgres"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            durable: true,
            push: "poll",
            lease: "ttl",
        }
    }

    async fn init_schema(&self) -> Result<()> {
        self.client.batch_execute(SCHEMA).await?;
        Ok(())
    }

    async fn ensure_address(
        &self,
        address: &str,
        description: Option<&str>,
        scope: Option<&str>,
        tags: Option<&str>,
    ) -> Result<()> {
        self.client
            .execute(
                "INSERT INTO addresses(address, description, scope, tags, status, created_at_ms) \
                 VALUES ($1,$2,$3,$4,'active',$5) \
                 ON CONFLICT(address) DO UPDATE SET \
                    description=COALESCE(excluded.description, addresses.description), \
                    scope=COALESCE(excluded.scope, addresses.scope), \
                    tags=COALESCE(excluded.tags, addresses.tags)",
                &[&address, &description, &scope, &tags, &now_ms()],
            )
            .await?;
        Ok(())
    }

    async fn get_address(&self, address: &str) -> Result<Option<AddressRow>> {
        let row = self
            .client
            .query_opt(
                "SELECT address, description, scope, tags, status, created_at_ms \
                 FROM addresses WHERE address=$1",
                &[&address],
            )
            .await?;
        Ok(row.map(|r| map_address(&r)))
    }

    async fn set_address_status(&self, address: &str, status: &str) -> Result<bool> {
        let n = self
            .client
            .execute(
                "UPDATE addresses SET status=$2 WHERE address=$1",
                &[&address, &status],
            )
            .await?;
        Ok(n > 0)
    }

    async fn list_addresses(
        &self,
        scope: Option<&str>,
        include_retired: bool,
    ) -> Result<Vec<AddressRow>> {
        let mut sql = String::from(
            "SELECT address, description, scope, tags, status, created_at_ms FROM addresses WHERE TRUE",
        );
        if !include_retired {
            sql.push_str(" AND status='active'");
        }
        let rows = if let Some(s) = scope {
            sql.push_str(" AND scope=$1 ORDER BY address");
            self.client.query(&sql, &[&s]).await?
        } else {
            sql.push_str(" ORDER BY address");
            self.client.query(&sql, &[]).await?
        };
        Ok(rows.iter().map(map_address).collect())
    }

    async fn claim_lease(&self, claim: &LeaseClaim, window_secs: i64) -> Result<LeaseOutcome> {
        let now = now_ms();
        let live_floor = now - window_secs * 1000;
        let rows = self
            .client
            .query(
                "INSERT INTO leases(address, occupant, host, principal, description, tags, scope, pid, since_ms, heartbeat_at_ms) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$9) \
                 ON CONFLICT(address) DO UPDATE SET occupant=excluded.occupant, host=excluded.host, \
                    principal=excluded.principal, description=excluded.description, tags=excluded.tags, \
                    scope=excluded.scope, pid=excluded.pid, \
                    since_ms = CASE WHEN leases.occupant = excluded.occupant THEN leases.since_ms ELSE excluded.since_ms END, \
                    heartbeat_at_ms=excluded.heartbeat_at_ms \
                 WHERE leases.occupant = excluded.occupant OR leases.heartbeat_at_ms < $10 \
                 RETURNING address",
                &[
                    &claim.address, &claim.occupant, &claim.host, &claim.principal,
                    &claim.description, &claim.tags, &claim.scope, &claim.pid, &now, &live_floor,
                ],
            )
            .await?;
        if rows.is_empty() {
            // Conflict and not claimable: report the current live occupant.
            let lease = self.get_lease(&claim.address).await?;
            Ok(LeaseOutcome::AlreadyOccupied(lease.ok_or_else(|| {
                anyhow!("lease claim blocked but lease row vanished")
            })?))
        } else {
            Ok(LeaseOutcome::Claimed)
        }
    }

    async fn heartbeat(&self, address: &str) -> Result<()> {
        self.client
            .execute(
                "UPDATE leases SET heartbeat_at_ms=$2 WHERE address=$1",
                &[&address, &now_ms()],
            )
            .await?;
        Ok(())
    }

    async fn release_lease(&self, address: &str, occupant: &str) -> Result<bool> {
        let n = self
            .client
            .execute(
                "DELETE FROM leases WHERE address=$1 AND (occupant=$2 OR occupant IS NULL)",
                &[&address, &occupant],
            )
            .await?;
        Ok(n > 0)
    }

    async fn get_lease(&self, address: &str) -> Result<Option<LeaseRow>> {
        let row = self
            .client
            .query_opt(
                "SELECT address, occupant, host, principal, description, tags, scope, pid, since_ms, heartbeat_at_ms \
                 FROM leases WHERE address=$1",
                &[&address],
            )
            .await?;
        Ok(row.map(|r| map_lease(&r)))
    }

    async fn occupancy(&self, address: &str, window_secs: i64) -> Result<Occupancy> {
        let now = now_ms();
        let row = self
            .client
            .query_opt(
                "SELECT occupant, heartbeat_at_ms FROM leases WHERE address=$1",
                &[&address],
            )
            .await?;
        Ok(match row {
            None => Occupancy {
                occupied: false,
                age_secs: 0.0,
                occupant: None,
            },
            Some(r) => {
                let occupant: Option<String> = r.get("occupant");
                let hb: i64 = r.get("heartbeat_at_ms");
                let age_ms = now - hb;
                Occupancy {
                    occupied: age_ms < window_secs * 1000,
                    age_secs: age_ms as f64 / 1000.0,
                    occupant,
                }
            }
        })
    }

    async fn max_id(&self, address: &str) -> Result<i64> {
        Ok(self
            .client
            .query_one(
                "SELECT COALESCE(MAX(id),0) AS m FROM messages WHERE to_addr=$1",
                &[&address],
            )
            .await?
            .get("m"))
    }

    async fn fetch_after(&self, address: &str, cursor: i64) -> Result<Vec<MessageRow>> {
        let sql = format!("SELECT {MSG_COLS} FROM messages WHERE to_addr=$1 AND id>$2 ORDER BY id");
        let rows = self.client.query(&sql, &[&address, &cursor]).await?;
        Ok(rows.iter().map(map_message).collect())
    }

    async fn insert_message(&self, m: &NewMessage) -> Result<MessageRow> {
        let now = now_ms();
        // Determine the parent's thread first (NULL for a root message).
        let parent_thread: Option<i64> = match m.parent_id {
            Some(pid) => self
                .client
                .query_opt(
                    "SELECT COALESCE(thread_id, id) AS t FROM messages WHERE id=$1",
                    &[&pid],
                )
                .await?
                .map(|r| r.get::<_, i64>("t")),
            None => None,
        };
        let id: i64 = self
            .client
            .query_one(
                "INSERT INTO messages(thread_id, parent_id, from_addr, to_addr, cc, kind, attention, \
                    requires_disposition, subject, body, metadata, sent_at_ms, created_at_ms) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) RETURNING id",
                &[
                    &parent_thread, &m.parent_id, &m.from_addr, &m.to_addr, &m.cc, &m.kind,
                    &m.attention.as_str(), &m.requires_disposition, &m.subject, &m.body,
                    &m.metadata, &m.sent_at_ms, &now,
                ],
            )
            .await?
            .get("id");
        if parent_thread.is_none() {
            self.client
                .execute("UPDATE messages SET thread_id=$1 WHERE id=$1", &[&id])
                .await?;
        }
        let row = self
            .client
            .query_one(
                &format!("SELECT {MSG_COLS} FROM messages WHERE id=$1"),
                &[&id],
            )
            .await?;
        Ok(map_message(&row))
    }

    async fn get_message(&self, id: i64) -> Result<Option<MessageRow>> {
        let row = self
            .client
            .query_opt(
                &format!("SELECT {MSG_COLS} FROM messages WHERE id=$1"),
                &[&id],
            )
            .await?;
        Ok(row.map(|r| map_message(&r)))
    }

    async fn thread_messages(&self, thread_id: i64) -> Result<Vec<MessageRow>> {
        let sql =
            format!("SELECT {MSG_COLS} FROM messages WHERE thread_id=$1 OR id=$1 ORDER BY id");
        let rows = self.client.query(&sql, &[&thread_id]).await?;
        Ok(rows.iter().map(map_message).collect())
    }

    async fn inbox(&self, address: &str, include_all: bool, limit: i64) -> Result<Vec<InboxItem>> {
        let sql = format!(
            "SELECT {MSG_COLS}, \
                (SELECT d.state FROM dispositions d WHERE d.message_id=messages.id \
                   AND d.recipient=$1 ORDER BY d.id DESC LIMIT 1) AS latest_disp \
             FROM messages WHERE to_addr=$1 ORDER BY id DESC LIMIT $2"
        );
        let rows = self.client.query(&sql, &[&address, &limit]).await?;
        let items: Vec<InboxItem> = rows
            .iter()
            .map(|r| {
                let message = map_message(r);
                let latest: Option<String> = r.get("latest_disp");
                let terminal = latest
                    .as_deref()
                    .map(Disposition::is_terminal_str)
                    .unwrap_or(false);
                let actionable = message.requires_disposition && !terminal;
                InboxItem {
                    message,
                    latest_disposition: latest,
                    actionable,
                }
            })
            .filter(|it| include_all || it.actionable)
            .collect();
        Ok(items)
    }

    async fn export(
        &self,
        address: Option<&str>,
        thread: Option<i64>,
        since: i64,
    ) -> Result<Vec<MessageRow>> {
        let mut sql = format!("SELECT {MSG_COLS} FROM messages WHERE id>$1");
        if address.is_some() {
            sql.push_str(" AND (to_addr=$2 OR from_addr=$2)");
        }
        if let Some(t) = thread {
            sql.push_str(&format!(" AND (thread_id={t} OR id={t})"));
        }
        sql.push_str(" ORDER BY id");
        let rows = if let Some(addr) = address {
            self.client.query(&sql, &[&since, &addr]).await?
        } else {
            self.client.query(&sql, &[&since]).await?
        };
        Ok(rows.iter().map(map_message).collect())
    }

    async fn insert_disposition(
        &self,
        message_id: i64,
        recipient: &str,
        state: &str,
        note: Option<&str>,
        by: Option<&str>,
    ) -> Result<DispositionRow> {
        let now = now_ms();
        let id: i64 = self
            .client
            .query_one(
                "INSERT INTO dispositions(message_id, recipient, state, note, by_principal, at_ms) \
                 VALUES ($1,$2,$3,$4,$5,$6) RETURNING id",
                &[&message_id, &recipient, &state, &note, &by, &now],
            )
            .await?
            .get("id");
        Ok(DispositionRow {
            id,
            message_id,
            recipient: recipient.to_string(),
            state: state.to_string(),
            note: note.map(str::to_string),
            by_principal: by.map(str::to_string),
            at_ms: now,
        })
    }

    async fn dispositions_for(&self, message_id: i64) -> Result<Vec<DispositionRow>> {
        let rows = self
            .client
            .query(
                "SELECT id, message_id, recipient, state, note, by_principal, at_ms \
                 FROM dispositions WHERE message_id=$1 ORDER BY id",
                &[&message_id],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| DispositionRow {
                id: r.get("id"),
                message_id: r.get("message_id"),
                recipient: r.get("recipient"),
                state: r.get("state"),
                note: r.get("note"),
                by_principal: r.get("by_principal"),
                at_ms: r.get("at_ms"),
            })
            .collect())
    }

    async fn notify_new(&self, address: &str, id: i64, sent_at_ms: i64) -> Result<()> {
        let payload =
            serde_json::json!({"address": address, "id": id, "sent_at_ms": sent_at_ms}).to_string();
        self.client
            .execute("SELECT pg_notify($1,$2)", &[&NOTIFY_CHANNEL, &payload])
            .await?;
        Ok(())
    }
}
