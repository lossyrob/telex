//! Postgres backend: the networked substrate. Same semantic model as SQLite, with
//! epoch-ms integer timestamps for parity. v0 uses TTL-heartbeat liveness and
//! poll-with-cursor delivery; LISTEN/NOTIFY push is a best-effort extra. Connection
//! config and credentials come from a backend profile (see `profiles`), not the env.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use tokio_postgres::Row;

use super::{Backend, Capabilities};
use crate::model::*;

pub const NOTIFY_CHANNEL: &str = "telex_messages";

pub struct PgBackend {
    client: tokio_postgres::Client,
}

pub fn make_tls() -> Result<postgres_native_tls::MakeTlsConnector> {
    let tls = native_tls::TlsConnector::builder()
        .build()
        .context("building TLS connector")?;
    Ok(postgres_native_tls::MakeTlsConnector::new(tls))
}

/// Allow only a safe SQL identifier for a schema name (no injection via search_path).
pub fn sanitize_ident(s: &str) -> Result<String> {
    // Postgres truncates identifiers to NAMEDATALEN-1 (63) bytes, so anything longer would be
    // silently shortened — a footgun that can collide two distinct schema names. Reject it.
    let valid = !s.is_empty()
        && s.len() <= 63
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && s.chars()
            .next()
            .map(|c| !c.is_ascii_digit())
            .unwrap_or(false);
    if valid {
        Ok(s.to_string())
    } else {
        anyhow::bail!(
            "invalid schema '{s}' (use 1-63 chars: letters, digits, underscore; not a leading digit)"
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
CREATE TABLE IF NOT EXISTS deliveries (
    id              bigserial PRIMARY KEY,
    message_id      bigint NOT NULL,
    recipient       text NOT NULL,
    occupant        text,
    delivered_at_ms bigint NOT NULL,
    UNIQUE(message_id, recipient)
);
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
        lease_epoch: None,
        owner_instance_id: None,
    }
}

impl PgBackend {
    /// Connect using a fully-built config (host/user/db/password) and an optional schema
    /// to isolate telex tables in. The password is resolved by the caller (profile).
    pub async fn connect_with(
        config: tokio_postgres::Config,
        schema: Option<&str>,
    ) -> Result<Self> {
        let (client, connection) = config
            .connect(make_tls()?)
            .await
            .context("connecting to postgres")?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("[telex] postgres connection ended: {e}");
            }
        });
        // The holder's live drain (`fetch_undelivered`) is correct only if every poll re-snapshots
        // the latest committed state — i.e. each autocommit query runs under READ COMMITTED. A
        // server- or role-level `default_transaction_isolation` of REPEATABLE READ/SERIALIZABLE
        // (a one-liner on managed Postgres) would otherwise freeze the snapshot and re-open the
        // issue #18 race (a frozen snapshot cannot see a later-committing lower id). Pin it on the
        // session so the guarantee does not depend on external configuration; telex never drains
        // inside a long-lived transaction. See DECISIONS 0013.
        client
            .batch_execute(
                "SET SESSION CHARACTERISTICS AS TRANSACTION ISOLATION LEVEL READ COMMITTED",
            )
            .await
            .context("pinning READ COMMITTED isolation")?;
        if let Some(s) = schema {
            let s = sanitize_ident(s)?;
            client
                .batch_execute(&format!(
                    "CREATE SCHEMA IF NOT EXISTS {s}; SET search_path TO {s}, public;"
                ))
                .await
                .context("setting schema search_path")?;
        }
        Ok(Self { client })
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

    async fn mark_delivered(
        &self,
        message_id: i64,
        recipient: &str,
        occupant: Option<&str>,
    ) -> Result<()> {
        let now = now_ms();
        self.client
            .execute(
                "INSERT INTO deliveries(message_id, recipient, occupant, delivered_at_ms) \
                 VALUES ($1,$2,$3,$4) ON CONFLICT (message_id, recipient) DO NOTHING",
                &[&message_id, &recipient, &occupant, &now],
            )
            .await?;
        Ok(())
    }

    async fn fetch_undelivered(&self, address: &str) -> Result<Vec<MessageRow>> {
        // Undelivered = messages addressed here with no delivery record AND whose latest disposition
        // for this recipient is not terminal, ordered by id. There is deliberately NO id floor: the
        // holder's live drain queues exactly this set (deduped in-memory), so a concurrently-
        // committed lower id — which by definition has no delivery record — is delivered live and is
        // never skipped by a high-water cursor (issue #18 / DECISIONS 0013). Cost is O(address
        // history) per call rather than O(new); acceptable at this scale, and a safe id floor is
        // deferred because a naive one would re-open exactly this gap (see 0013).
        let sql = format!(
            "SELECT {MSG_COLS} FROM messages m \
             WHERE m.to_addr=$1 \
               AND NOT EXISTS (SELECT 1 FROM deliveries d \
                               WHERE d.message_id=m.id AND d.recipient=$1) \
               AND COALESCE((SELECT disp.state FROM dispositions disp \
                             WHERE disp.message_id=m.id AND disp.recipient=$1 \
                             ORDER BY disp.id DESC LIMIT 1), '') NOT IN ({}) \
             ORDER BY m.id",
            terminal_dispositions_sql_list()
        );
        let rows = self.client.query(&sql, &[&address]).await?;
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
             FROM messages WHERE to_addr=$1 OR cc LIKE '%' || $1 || '%' ORDER BY id DESC LIMIT $2"
        );
        let rows = self.client.query(&sql, &[&address, &limit]).await?;
        let items: Vec<InboxItem> = rows
            .iter()
            .map(|r| {
                let message = map_message(r);
                let latest: Option<String> = r.get("latest_disp");
                let delivered_to = address.to_string();
                let primary_to = message.to_addr.clone();
                let cc = cc_recipients(message.cc.as_deref());
                let role = delivery_role(&delivered_to, &primary_to, message.cc.as_deref());
                let requires_for_recipient = requires_disposition_for_recipient(
                    message.requires_disposition,
                    &delivered_to,
                    &primary_to,
                );
                let terminal = latest
                    .as_deref()
                    .map(Disposition::is_terminal_str)
                    .unwrap_or(false);
                let actionable = requires_for_recipient && !terminal;
                InboxItem {
                    message,
                    delivered_to,
                    primary_to,
                    cc_recipients: cc,
                    delivery_role: role.to_string(),
                    requires_disposition_for_current_recipient: requires_for_recipient,
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

#[cfg(test)]
mod tests {
    use super::sanitize_ident;

    #[test]
    fn sanitize_ident_accepts_valid_names() {
        for s in ["telex", "telex_conformance", "s_1_2", "A9"] {
            assert_eq!(sanitize_ident(s).unwrap(), s);
        }
    }

    #[test]
    fn sanitize_ident_rejects_invalid_names() {
        // Empty, leading digit, illegal chars / injection attempts.
        for s in ["", "1abc", "a-b", "a;b", "a b", "public.bad", "a\"b"] {
            assert!(sanitize_ident(s).is_err(), "should reject {s:?}");
        }
    }

    #[test]
    fn sanitize_ident_enforces_63_byte_limit() {
        let max = "a".repeat(63);
        assert_eq!(sanitize_ident(&max).unwrap(), max);
        let over = "a".repeat(64);
        assert!(
            sanitize_ident(&over).is_err(),
            "identifiers over 63 bytes must be rejected, not silently truncated"
        );
    }
}
