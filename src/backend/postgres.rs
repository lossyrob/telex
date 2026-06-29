//! Postgres backend: the networked substrate. Same semantic model as SQLite, with
//! epoch-ms integer timestamps for parity. The daemon owns the LISTEN/NOTIFY
//! receive side; the backend keeps poll + explicit Ack as the durable correctness path.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use tokio::sync::Mutex as AsyncMutex;
use tokio_postgres::{Row, Transaction};

use super::{Backend, Capabilities};
use crate::model::*;

pub struct PgBackend {
    client: AsyncMutex<tokio_postgres::Client>,
    notify_channel: String,
}

pub fn notify_channel_for_schema(schema: Option<&str>) -> Result<String> {
    let schema = schema.unwrap_or("public");
    if schema != "public" {
        sanitize_ident(schema)?;
    }
    Ok(format!(
        "telex_messages_{:016x}",
        fnv1a64(schema.as_bytes())
    ))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
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
    heartbeat_at_ms bigint NOT NULL,
    lease_epoch     bigint,
    owner_instance_id text
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
    consumed_at_ms  bigint,
    UNIQUE(message_id, recipient)
);
CREATE TABLE IF NOT EXISTS telex_schema_meta (
    key   text PRIMARY KEY,
    value text NOT NULL
);
CREATE TABLE IF NOT EXISTS detach_tombstones (
    session_id text NOT NULL,
    address    text NOT NULL,
    reason     text NOT NULL,
    at_ms      bigint NOT NULL,
    PRIMARY KEY(session_id, address)
);
CREATE INDEX IF NOT EXISTS detach_tombstones_session_idx
    ON detach_tombstones(session_id);
"#;

const MSG_COLS: &str = "id, thread_id, parent_id, from_addr, to_addr, cc, kind, attention, \
    requires_disposition, subject, body, metadata, sent_at_ms, created_at_ms";
const MSG_COLS_M: &str = "m.id, m.thread_id, m.parent_id, m.from_addr, m.to_addr, m.cc, m.kind, \
    m.attention, m.requires_disposition, m.subject, m.body, m.metadata, m.sent_at_ms, \
    m.created_at_ms";

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
        lease_epoch: r.get("lease_epoch"),
        owner_instance_id: r.get("owner_instance_id"),
    }
}

fn fanout_recipients(to_addr: &str, cc: Option<&str>) -> Vec<String> {
    let mut recipients = vec![to_addr.to_string()];
    for recipient in cc_recipients(cc) {
        if !recipients.iter().any(|r| r == &recipient) {
            recipients.push(recipient);
        }
    }
    recipients
}

async fn pg_now_ms(client: &tokio_postgres::Client) -> Result<i64> {
    Ok(client
        .query_one(
            "SELECT floor(extract(epoch from clock_timestamp()) * 1000)::bigint",
            &[],
        )
        .await?
        .get(0))
}

async fn pg_tx_now_ms(tx: &Transaction<'_>) -> Result<i64> {
    Ok(tx
        .query_one(
            "SELECT floor(extract(epoch from clock_timestamp()) * 1000)::bigint",
            &[],
        )
        .await?
        .get(0))
}

async fn materialize_pending_delivery_rows_for_recipient(
    client: &tokio_postgres::Client,
    recipient: &str,
) -> Result<()> {
    client
        .execute(
            "INSERT INTO deliveries(message_id, recipient, delivered_at_ms, consumed_at_ms)
             SELECT m.id,
                    $1,
                    m.created_at_ms,
                    CASE WHEN m.to_addr = $1 THEN NULL ELSE m.created_at_ms END
             FROM messages m
             WHERE m.to_addr=$1
               AND NOT EXISTS (
                   SELECT 1 FROM deliveries d
                   WHERE d.message_id=m.id AND d.recipient=$1
               )
             ON CONFLICT(message_id, recipient) DO NOTHING",
            &[&recipient],
        )
        .await?;
    Ok(())
}

async fn materialize_pending_delivery_rows_for_recipient_tx(
    tx: &Transaction<'_>,
    recipient: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO deliveries(message_id, recipient, delivered_at_ms, consumed_at_ms)
         SELECT m.id,
                $1,
                m.created_at_ms,
                CASE WHEN m.to_addr = $1 THEN NULL ELSE m.created_at_ms END
         FROM messages m
         WHERE m.to_addr=$1
           AND NOT EXISTS (
               SELECT 1 FROM deliveries d
               WHERE d.message_id=m.id AND d.recipient=$1
           )
         ON CONFLICT(message_id, recipient) DO NOTHING",
        &[&recipient],
    )
    .await?;
    Ok(())
}

async fn backfill_existing_deliveries_consumed_once(
    client: &mut tokio_postgres::Client,
) -> Result<()> {
    let tx = client.transaction().await?;
    let complete: bool = tx
        .query_one(
            "SELECT EXISTS(
                SELECT 1 FROM telex_schema_meta
                WHERE key='delivery_consumed_backfill_v1_complete' AND value='1'
             )",
            &[],
        )
        .await?
        .get(0);
    if !complete {
        tx.execute(
            "UPDATE deliveries
             SET consumed_at_ms = delivered_at_ms
             WHERE consumed_at_ms IS NULL",
            &[],
        )
        .await?;
        tx.execute(
            "INSERT INTO telex_schema_meta(key, value)
             VALUES ('delivery_consumed_backfill_v1_complete', '1')
             ON CONFLICT(key) DO UPDATE SET value='1'",
            &[],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
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
        let notify_channel = notify_channel_for_schema(schema)?;
        Ok(Self {
            client: AsyncMutex::new(client),
            notify_channel,
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
            push: "native",
            lease: "ttl",
        }
    }

    async fn init_schema(&self) -> Result<()> {
        let mut client = self.client.lock().await;
        client.batch_execute(SCHEMA).await?;
        client
            .batch_execute(
                "ALTER TABLE leases ADD COLUMN IF NOT EXISTS lease_epoch bigint;
                 ALTER TABLE leases ADD COLUMN IF NOT EXISTS owner_instance_id text;
                 ALTER TABLE deliveries ADD COLUMN IF NOT EXISTS consumed_at_ms bigint;
                 CREATE INDEX IF NOT EXISTS deliveries_recipient_pending_idx
                    ON deliveries(recipient, consumed_at_ms, message_id);
                 CREATE TABLE IF NOT EXISTS telex_schema_meta (
                    key   text PRIMARY KEY,
                    value text NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS detach_tombstones (
                    session_id text NOT NULL,
                    address    text NOT NULL,
                    reason     text NOT NULL,
                    at_ms      bigint NOT NULL,
                    PRIMARY KEY(session_id, address)
                 );
                 CREATE INDEX IF NOT EXISTS detach_tombstones_session_idx
                    ON detach_tombstones(session_id);",
            )
            .await?;
        backfill_existing_deliveries_consumed_once(&mut client).await?;
        Ok(())
    }

    async fn ensure_address(
        &self,
        address: &str,
        description: Option<&str>,
        scope: Option<&str>,
        tags: Option<&str>,
    ) -> Result<()> {
        let client = self.client.lock().await;
        client
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
        let client = self.client.lock().await;
        let row = client
            .query_opt(
                "SELECT address, description, scope, tags, status, created_at_ms \
                 FROM addresses WHERE address=$1",
                &[&address],
            )
            .await?;
        Ok(row.map(|r| map_address(&r)))
    }

    async fn set_address_status(&self, address: &str, status: &str) -> Result<bool> {
        let client = self.client.lock().await;
        let n = client
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
            let client = self.client.lock().await;
            client.query(&sql, &[&s]).await?
        } else {
            sql.push_str(" ORDER BY address");
            let client = self.client.lock().await;
            client.query(&sql, &[]).await?
        };
        Ok(rows.iter().map(map_address).collect())
    }

    async fn claim_lease(&self, claim: &LeaseClaim, window_secs: i64) -> Result<LeaseOutcome> {
        let client = self.client.lock().await;
        let now = pg_now_ms(&client).await?;
        let live_floor = now - window_secs * 1000;
        let rows = client
            .query(
                "INSERT INTO leases(address, occupant, host, principal, description, tags, scope, pid, since_ms, heartbeat_at_ms, lease_epoch, owner_instance_id) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$9,1,$2) \
                 ON CONFLICT(address) DO UPDATE SET occupant=excluded.occupant, host=excluded.host, \
                    principal=excluded.principal, description=excluded.description, tags=excluded.tags, \
                    scope=excluded.scope, pid=excluded.pid, \
                    since_ms = CASE WHEN leases.occupant = excluded.occupant THEN leases.since_ms ELSE excluded.since_ms END, \
                    heartbeat_at_ms=excluded.heartbeat_at_ms, \
                    lease_epoch = CASE WHEN leases.occupant = excluded.occupant THEN COALESCE(leases.lease_epoch, 1) ELSE COALESCE(leases.lease_epoch, 0) + 1 END, \
                    owner_instance_id=excluded.owner_instance_id \
                 WHERE leases.occupant = excluded.occupant OR leases.owner_instance_id IS NULL OR leases.heartbeat_at_ms < $10 \
                 RETURNING address",
                &[
                    &claim.address, &claim.occupant, &claim.host, &claim.principal,
                    &claim.description, &claim.tags, &claim.scope, &claim.pid, &now, &live_floor,
                ],
            )
            .await?;
        if rows.is_empty() {
            // Conflict and not claimable: report the current live occupant.
            let lease = client
                .query_opt(
                    "SELECT address, occupant, host, principal, description, tags, scope, pid, since_ms, heartbeat_at_ms, lease_epoch, owner_instance_id \
                     FROM leases WHERE address=$1",
                    &[&claim.address],
                )
                .await?
                .map(|r| map_lease(&r));
            Ok(LeaseOutcome::AlreadyOccupied(lease.ok_or_else(|| {
                anyhow!("lease claim blocked but lease row vanished")
            })?))
        } else {
            Ok(LeaseOutcome::Claimed)
        }
    }

    async fn heartbeat(&self, address: &str) -> Result<()> {
        let client = self.client.lock().await;
        let now = pg_now_ms(&client).await?;
        client
            .execute(
                "UPDATE leases SET heartbeat_at_ms=$2 WHERE address=$1",
                &[&address, &now],
            )
            .await?;
        Ok(())
    }

    async fn release_lease(&self, address: &str, occupant: &str) -> Result<bool> {
        let client = self.client.lock().await;
        let n = client
            .execute(
                "UPDATE leases
                    SET owner_instance_id = NULL,
                        occupant = NULL,
                        heartbeat_at_ms = 0
                  WHERE address=$1 AND occupant=$2",
                &[&address, &occupant],
            )
            .await?;
        Ok(n > 0)
    }

    async fn get_lease(&self, address: &str) -> Result<Option<LeaseRow>> {
        let client = self.client.lock().await;
        let row = client
            .query_opt(
                "SELECT address, occupant, host, principal, description, tags, scope, pid, since_ms, heartbeat_at_ms, lease_epoch, owner_instance_id \
                 FROM leases WHERE address=$1",
                &[&address],
            )
            .await?;
        Ok(row.map(|r| map_lease(&r)))
    }

    async fn occupancy(&self, address: &str, window_secs: i64) -> Result<Occupancy> {
        let client = self.client.lock().await;
        let now = pg_now_ms(&client).await?;
        let row = client
            .query_opt(
                "SELECT occupant, heartbeat_at_ms, owner_instance_id FROM leases WHERE address=$1",
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
                let owner: Option<String> = r.get("owner_instance_id");
                let age_ms = now - hb;
                Occupancy {
                    occupied: owner.is_some() && age_ms < window_secs * 1000,
                    age_secs: age_ms as f64 / 1000.0,
                    occupant,
                }
            }
        })
    }

    async fn claim_epoch_lease(
        &self,
        address: &str,
        owner_instance_id: &str,
        liveness_window_secs: i64,
    ) -> Result<EpochClaimResult> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await?;
        let current = tx
            .query_opt(
                "SELECT lease_epoch, owner_instance_id, heartbeat_at_ms
                 FROM leases
                 WHERE address=$1
                 FOR UPDATE",
                &[&address],
            )
            .await?;

        let result = if let Some(row) = current {
            let lease_epoch: Option<i64> = row.get("lease_epoch");
            let current_owner: Option<String> = row.get("owner_instance_id");
            let heartbeat_at_ms: i64 = row.get("heartbeat_at_ms");

            if lease_epoch.is_none() {
                let now = pg_tx_now_ms(&tx).await?;
                let rows = tx
                    .query(
                        "UPDATE leases
                            SET owner_instance_id=$2,
                                lease_epoch=1,
                                heartbeat_at_ms=$3
                          WHERE address=$1 AND lease_epoch IS NULL
                          RETURNING lease_epoch, owner_instance_id",
                        &[&address, &owner_instance_id, &now],
                    )
                    .await?;
                if let Some(row) = rows.first() {
                    EpochClaimResult::Claimed(EpochClaimed {
                        lease_epoch: row.get("lease_epoch"),
                        owner_instance_id: row.get("owner_instance_id"),
                        legacy_cutover: true,
                    })
                } else {
                    let lease_row = tx
                        .query_one(
                            "SELECT address, occupant, host, principal, description, tags, scope, pid, since_ms, heartbeat_at_ms, lease_epoch, owner_instance_id
                             FROM leases WHERE address=$1",
                            &[&address],
                        )
                        .await?;
                    let lease_row = map_lease(&lease_row);
                    EpochClaimResult::AlreadyOwned {
                        lease_epoch: lease_row.lease_epoch.unwrap_or(0),
                        owner_instance_id: lease_row.owner_instance_id.clone().unwrap_or_default(),
                        lease_row,
                    }
                }
            } else {
                let lease_epoch = lease_epoch.unwrap();
                let now = pg_tx_now_ms(&tx).await?;
                let stale_cutoff = now - liveness_window_secs.max(0) * 1000;
                if current_owner.is_none() || heartbeat_at_ms < stale_cutoff {
                    let rows = tx
                        .query(
                            "UPDATE leases
                                SET owner_instance_id=$2,
                                    lease_epoch=lease_epoch + 1,
                                    heartbeat_at_ms=$3
                              WHERE address=$1
                                AND lease_epoch=$4
                                AND owner_instance_id IS NOT DISTINCT FROM $5
                                AND (owner_instance_id IS NULL OR heartbeat_at_ms < $6)
                              RETURNING lease_epoch, owner_instance_id",
                            &[
                                &address,
                                &owner_instance_id,
                                &now,
                                &lease_epoch,
                                &current_owner,
                                &stale_cutoff,
                            ],
                        )
                        .await?;
                    if let Some(row) = rows.first() {
                        EpochClaimResult::Claimed(EpochClaimed {
                            lease_epoch: row.get("lease_epoch"),
                            owner_instance_id: row.get("owner_instance_id"),
                            legacy_cutover: false,
                        })
                    } else {
                        let lease_row = tx
                            .query_one(
                                "SELECT address, occupant, host, principal, description, tags, scope, pid, since_ms, heartbeat_at_ms, lease_epoch, owner_instance_id
                                 FROM leases WHERE address=$1",
                                &[&address],
                            )
                            .await?;
                        let lease_row = map_lease(&lease_row);
                        EpochClaimResult::AlreadyOwned {
                            lease_epoch: lease_row.lease_epoch.unwrap_or(lease_epoch),
                            owner_instance_id: lease_row
                                .owner_instance_id
                                .clone()
                                .unwrap_or_default(),
                            lease_row,
                        }
                    }
                } else {
                    let lease_row = tx
                        .query_one(
                            "SELECT address, occupant, host, principal, description, tags, scope, pid, since_ms, heartbeat_at_ms, lease_epoch, owner_instance_id
                             FROM leases WHERE address=$1",
                            &[&address],
                        )
                        .await?;
                    let lease_row = map_lease(&lease_row);
                    EpochClaimResult::AlreadyOwned {
                        lease_epoch,
                        owner_instance_id: current_owner.unwrap_or_default(),
                        lease_row,
                    }
                }
            }
        } else {
            let now = pg_tx_now_ms(&tx).await?;
            let rows = tx
                .query(
                    "INSERT INTO leases(address, since_ms, heartbeat_at_ms, lease_epoch, owner_instance_id)
                     VALUES ($1, $2, $2, 1, $3)
                     ON CONFLICT(address) DO NOTHING
                     RETURNING lease_epoch, owner_instance_id",
                    &[&address, &now, &owner_instance_id],
                )
                .await?;
            if let Some(row) = rows.first() {
                EpochClaimResult::Claimed(EpochClaimed {
                    lease_epoch: row.get("lease_epoch"),
                    owner_instance_id: row.get("owner_instance_id"),
                    legacy_cutover: false,
                })
            } else {
                let lease_row = tx
                    .query_one(
                        "SELECT address, occupant, host, principal, description, tags, scope, pid, since_ms, heartbeat_at_ms, lease_epoch, owner_instance_id
                         FROM leases WHERE address=$1",
                        &[&address],
                    )
                    .await?;
                let lease_row = map_lease(&lease_row);
                EpochClaimResult::AlreadyOwned {
                    lease_epoch: lease_row.lease_epoch.unwrap_or(1),
                    owner_instance_id: lease_row.owner_instance_id.clone().unwrap_or_default(),
                    lease_row,
                }
            }
        };
        tx.commit().await?;
        Ok(result)
    }

    async fn heartbeat_epoch(
        &self,
        address: &str,
        owner_instance_id: &str,
        lease_epoch: i64,
    ) -> Result<bool> {
        let client = self.client.lock().await;
        let now = pg_now_ms(&client).await?;
        let n = client
            .execute(
                "UPDATE leases
                    SET heartbeat_at_ms=$4
                  WHERE address=$1 AND owner_instance_id=$2 AND lease_epoch=$3",
                &[&address, &owner_instance_id, &lease_epoch, &now],
            )
            .await?;
        Ok(n > 0)
    }

    async fn release_epoch_lease(
        &self,
        address: &str,
        owner_instance_id: &str,
        lease_epoch: i64,
    ) -> Result<bool> {
        let client = self.client.lock().await;
        let n = client
            .execute(
                "UPDATE leases
                    SET owner_instance_id = NULL
                  WHERE address=$1 AND owner_instance_id=$2 AND lease_epoch=$3",
                &[&address, &owner_instance_id, &lease_epoch],
            )
            .await?;
        Ok(n > 0)
    }

    async fn release_epoch_lease_for_detach(
        &self,
        address: &str,
        owner_instance_id: &str,
        lease_epoch: i64,
        session_id: &str,
        reason: &str,
    ) -> Result<bool> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await?;
        let n = tx
            .execute(
                "UPDATE leases
                    SET owner_instance_id = NULL
                  WHERE address=$1 AND owner_instance_id=$2 AND lease_epoch=$3",
                &[&address, &owner_instance_id, &lease_epoch],
            )
            .await?;
        if n > 0 {
            let now = pg_tx_now_ms(&tx).await?;
            tx.execute(
                "INSERT INTO detach_tombstones(session_id, address, reason, at_ms)
                 VALUES ($1,$2,$3,$4)
                 ON CONFLICT(session_id, address) DO UPDATE SET
                    reason=excluded.reason,
                    at_ms=excluded.at_ms",
                &[&session_id, &address, &reason, &now],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(n > 0)
    }

    async fn reset_epoch_lease(&self, address: &str) -> Result<Option<i64>> {
        let client = self.client.lock().await;
        let row = client
            .query_opt(
                "UPDATE leases
                    SET owner_instance_id = NULL,
                        heartbeat_at_ms = 0
                  WHERE address=$1
                  RETURNING lease_epoch",
                &[&address],
            )
            .await?;
        Ok(row.map(|r| r.get("lease_epoch")))
    }

    async fn mark_consumed_if_current_owner(
        &self,
        recipient: &str,
        owner_instance_id: &str,
        lease_epoch: i64,
        message_id: i64,
    ) -> Result<DeliveryOutcome> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await?;
        let lease = tx
            .query_opt(
                "SELECT lease_epoch, owner_instance_id
                 FROM leases
                 WHERE address=$1
                 FOR UPDATE",
                &[&recipient],
            )
            .await?;
        let is_owner = lease.is_some_and(|row| {
            let current_epoch: Option<i64> = row.get("lease_epoch");
            let current_owner: Option<String> = row.get("owner_instance_id");
            current_epoch == Some(lease_epoch)
                && current_owner.as_deref() == Some(owner_instance_id)
        });
        if !is_owner {
            tx.rollback().await?;
            return Ok(DeliveryOutcome::NotOwner);
        }

        materialize_pending_delivery_rows_for_recipient_tx(&tx, recipient).await?;
        let consumed = tx
            .query_opt(
                "SELECT consumed_at_ms
                 FROM deliveries
                 WHERE message_id=$1 AND recipient=$2",
                &[&message_id, &recipient],
            )
            .await?;
        let outcome = match consumed {
            None => DeliveryOutcome::AckNoOp,
            Some(row) => {
                let consumed_at_ms: Option<i64> = row.get("consumed_at_ms");
                if consumed_at_ms.is_some() {
                    DeliveryOutcome::AlreadyConsumed
                } else {
                    let now = pg_tx_now_ms(&tx).await?;
                    let n = tx
                        .execute(
                            "UPDATE deliveries
                                SET consumed_at_ms=$3
                              WHERE message_id=$1 AND recipient=$2 AND consumed_at_ms IS NULL",
                            &[&message_id, &recipient, &now],
                        )
                        .await?;
                    if n > 0 {
                        DeliveryOutcome::Marked
                    } else {
                        DeliveryOutcome::AlreadyConsumed
                    }
                }
            }
        };
        tx.commit().await?;
        Ok(outcome)
    }

    async fn durable_clock_now_ms(&self) -> Result<i64> {
        let client = self.client.lock().await;
        pg_now_ms(&client).await
    }

    async fn delivery_retention_count(&self) -> Result<i64> {
        let client = self.client.lock().await;
        Ok(client
            .query_one("SELECT COUNT(*) FROM deliveries", &[])
            .await?
            .get(0))
    }

    async fn pending_unconsumed_count(&self, address: &str) -> Result<i64> {
        let client = self.client.lock().await;
        materialize_pending_delivery_rows_for_recipient(&client, address).await?;
        let sql = format!(
            "SELECT COUNT(*) FROM deliveries d
             JOIN messages m ON m.id=d.message_id
             WHERE d.recipient=$1
               AND d.consumed_at_ms IS NULL
               AND COALESCE((SELECT disp.state FROM dispositions disp
                              WHERE disp.message_id=m.id AND disp.recipient=$1
                              ORDER BY disp.id DESC LIMIT 1), '') NOT IN ({})",
            terminal_dispositions_sql_list()
        );
        Ok(client.query_one(&sql, &[&address]).await?.get(0))
    }

    async fn record_detach_tombstone(
        &self,
        session_id: &str,
        address: &str,
        reason: &str,
    ) -> Result<()> {
        let client = self.client.lock().await;
        let now = pg_now_ms(&client).await?;
        client
            .execute(
                "INSERT INTO detach_tombstones(session_id, address, reason, at_ms)
                 VALUES ($1,$2,$3,$4)
                 ON CONFLICT(session_id, address) DO UPDATE SET
                    reason=excluded.reason,
                    at_ms=excluded.at_ms",
                &[&session_id, &address, &reason, &now],
            )
            .await?;
        Ok(())
    }

    async fn clear_detach_tombstone(&self, session_id: &str, address: &str) -> Result<()> {
        let client = self.client.lock().await;
        client
            .execute(
                "DELETE FROM detach_tombstones WHERE session_id=$1 AND address=$2",
                &[&session_id, &address],
            )
            .await?;
        Ok(())
    }

    async fn detach_tombstone(
        &self,
        session_id: &str,
        address: &str,
    ) -> Result<Option<DetachTombstone>> {
        let client = self.client.lock().await;
        let row = client
            .query_opt(
                "SELECT session_id, address, reason, at_ms
                 FROM detach_tombstones
                 WHERE session_id=$1 AND address=$2",
                &[&session_id, &address],
            )
            .await?;
        Ok(row.map(|r| DetachTombstone {
            session_id: r.get("session_id"),
            address: r.get("address"),
            reason: r.get("reason"),
            at_ms: r.get("at_ms"),
        }))
    }

    async fn mark_delivered(
        &self,
        message_id: i64,
        recipient: &str,
        occupant: Option<&str>,
    ) -> Result<()> {
        let client = self.client.lock().await;
        let now = pg_now_ms(&client).await?;
        client
            .execute(
                "INSERT INTO deliveries(message_id, recipient, occupant, delivered_at_ms, consumed_at_ms) \
                 VALUES ($1,$2,$3,$4,$4)
                 ON CONFLICT (message_id, recipient) DO UPDATE SET
                    occupant = COALESCE(excluded.occupant, deliveries.occupant),
                    consumed_at_ms = COALESCE(deliveries.consumed_at_ms, excluded.consumed_at_ms)",
                &[&message_id, &recipient, &occupant, &now],
            )
            .await?;
        Ok(())
    }

    async fn fetch_undelivered(&self, address: &str) -> Result<Vec<MessageRow>> {
        let client = self.client.lock().await;
        materialize_pending_delivery_rows_for_recipient(&client, address).await?;
        let sql = format!(
            "SELECT {MSG_COLS_M} FROM deliveries d
             JOIN messages m ON m.id=d.message_id
             WHERE d.recipient=$1
               AND d.consumed_at_ms IS NULL
               AND COALESCE((SELECT disp.state FROM dispositions disp
                             WHERE disp.message_id=m.id AND disp.recipient=$1
                             ORDER BY disp.id DESC LIMIT 1), '') NOT IN ({})
             ORDER BY d.message_id",
            terminal_dispositions_sql_list()
        );
        let rows = client.query(&sql, &[&address]).await?;
        Ok(rows.iter().map(map_message).collect())
    }

    async fn has_delivery_for_recipient(&self, message_id: i64, recipient: &str) -> Result<bool> {
        let client = self.client.lock().await;
        materialize_pending_delivery_rows_for_recipient(&client, recipient).await?;
        Ok(client
            .query_opt(
                "SELECT 1 FROM deliveries WHERE message_id=$1 AND recipient=$2 LIMIT 1",
                &[&message_id, &recipient],
            )
            .await?
            .is_some())
    }

    async fn insert_message(&self, m: &NewMessage) -> Result<MessageRow> {
        let now = now_ms();
        let mut client = self.client.lock().await;
        let tx = client.transaction().await?;
        // Determine the parent's thread first (NULL for a root message).
        let parent_thread: Option<i64> = match m.parent_id {
            Some(pid) => tx
                .query_opt(
                    "SELECT COALESCE(thread_id, id) AS t FROM messages WHERE id=$1",
                    &[&pid],
                )
                .await?
                .map(|r| r.get::<_, i64>("t")),
            None => None,
        };
        let id: i64 = tx
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
            tx.execute("UPDATE messages SET thread_id=$1 WHERE id=$1", &[&id])
                .await?;
        }
        for recipient in fanout_recipients(&m.to_addr, m.cc.as_deref()) {
            let consumed_at_ms = if recipient == m.to_addr {
                None
            } else {
                Some(now)
            };
            tx.execute(
                "INSERT INTO deliveries(message_id, recipient, delivered_at_ms, consumed_at_ms)
                 VALUES ($1,$2,$3,$4)
                 ON CONFLICT(message_id, recipient) DO NOTHING",
                &[&id, &recipient, &now, &consumed_at_ms],
            )
            .await?;
        }
        let row = tx
            .query_one(
                &format!("SELECT {MSG_COLS} FROM messages WHERE id=$1"),
                &[&id],
            )
            .await?;
        let message = map_message(&row);
        tx.commit().await?;
        Ok(message)
    }

    async fn get_message(&self, id: i64) -> Result<Option<MessageRow>> {
        let client = self.client.lock().await;
        let row = client
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
        let client = self.client.lock().await;
        let rows = client.query(&sql, &[&thread_id]).await?;
        Ok(rows.iter().map(map_message).collect())
    }

    async fn inbox(&self, address: &str, include_all: bool, limit: i64) -> Result<Vec<InboxItem>> {
        let sql = format!(
            "SELECT {MSG_COLS}, \
                (SELECT d.state FROM dispositions d WHERE d.message_id=messages.id \
                   AND d.recipient=$1 ORDER BY d.id DESC LIMIT 1) AS latest_disp \
             FROM messages WHERE to_addr=$1 OR cc LIKE '%' || $1 || '%' ORDER BY id DESC LIMIT $2"
        );
        let client = self.client.lock().await;
        let rows = client.query(&sql, &[&address, &limit]).await?;
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
            let client = self.client.lock().await;
            client.query(&sql, &[&since, &addr]).await?
        } else {
            let client = self.client.lock().await;
            client.query(&sql, &[&since]).await?
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
        let client = self.client.lock().await;
        let id: i64 = client
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
        let client = self.client.lock().await;
        let rows = client
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
        let client = self.client.lock().await;
        client
            .execute("SELECT pg_notify($1,$2)", &[&self.notify_channel, &payload])
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
