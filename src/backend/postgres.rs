//! Postgres backend: the networked substrate. Same semantic model as SQLite, with
//! epoch-ms integer timestamps for parity. The daemon owns the LISTEN/NOTIFY
//! receive side; the backend keeps poll + explicit Ack as the durable correctness path.

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use tokio::sync::{Mutex as AsyncMutex, MutexGuard};
use tokio_postgres::{Row, Transaction};

use super::{Backend, Capabilities, WaitCandidate, WaitFetchOptions};
use crate::model::*;

pub const CURRENT_SCHEMA_VERSION: i64 = 3;

pub struct PgBackend {
    connector: PgConnector,
    client: AsyncMutex<tokio_postgres::Client>,
    notify_channel: String,
}

enum PgConnector {
    Static {
        config: tokio_postgres::Config,
        schema: Option<String>,
    },
    Profile(crate::profiles::BackendProfile),
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
-- deliveries_recipient_pending_idx is created after the consumed_at_ms migration below.
-- Do not create it in this initial batch: on upgrade, CREATE TABLE IF NOT EXISTS is a
-- no-op for old deliveries tables, so indexing consumed_at_ms here fails before ALTER can add it.
CREATE TABLE IF NOT EXISTS clock_hwm (
    id     integer PRIMARY KEY CHECK (id = 1),
    hwm_ms bigint NOT NULL
);
CREATE TABLE IF NOT EXISTS telex_schema_meta (
    key   text PRIMARY KEY,
    value text NOT NULL
);
CREATE TABLE IF NOT EXISTS telex_schema_version (
    singleton integer NOT NULL DEFAULT 1 UNIQUE,
    version   bigint NOT NULL
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
CREATE TABLE IF NOT EXISTS application_operations (
    logical_store_id           text NOT NULL,
    application_responsibility text NOT NULL,
    operation_id               text NOT NULL,
    operation_kind             text NOT NULL,
    sender                     text NOT NULL,
    recipients_json            text NOT NULL,
    payload_fingerprint        text NOT NULL,
    retry_budget               bigint NOT NULL,
    state                      text NOT NULL,
    result_json                text,
    recovery_json              text,
    created_at_ms              bigint NOT NULL,
    updated_at_ms              bigint NOT NULL,
    completed_at_ms            bigint,
    PRIMARY KEY(logical_store_id, application_responsibility, operation_id)
);
CREATE INDEX IF NOT EXISTS application_operations_cleanup_idx
    ON application_operations(
        logical_store_id, application_responsibility, completed_at_ms, updated_at_ms
    );
CREATE TABLE IF NOT EXISTS application_operation_messages (
    logical_store_id           text NOT NULL,
    application_responsibility text NOT NULL,
    operation_id               text NOT NULL,
    message_id                 bigint NOT NULL,
    PRIMARY KEY(logical_store_id, application_responsibility, operation_id),
    UNIQUE(message_id)
);
CREATE TABLE IF NOT EXISTS application_store_identity (
    singleton integer PRIMARY KEY CHECK(singleton = 1),
    logical_store_id text NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS application_compound_steps (
    logical_store_id           text NOT NULL,
    application_responsibility text NOT NULL,
    operation_id               text NOT NULL,
    step_id                    text NOT NULL,
    position                   bigint NOT NULL,
    step_kind                  text NOT NULL,
    prerequisites_json         text NOT NULL,
    declaration_json           text NOT NULL,
    state                      text NOT NULL,
    outcome_json               text,
    recovery_json              text,
    created_at_ms              bigint NOT NULL,
    updated_at_ms              bigint NOT NULL,
    completed_at_ms            bigint,
    PRIMARY KEY(logical_store_id, application_responsibility, operation_id, step_id)
);
CREATE INDEX IF NOT EXISTS application_compound_steps_cleanup_idx
    ON application_compound_steps(
        logical_store_id, application_responsibility, completed_at_ms, updated_at_ms
    );
CREATE TABLE IF NOT EXISTS application_state_version (
    singleton integer PRIMARY KEY CHECK(singleton = 1),
    version   bigint NOT NULL
);
INSERT INTO application_state_version(singleton, version)
    VALUES (1, 0) ON CONFLICT(singleton) DO NOTHING;
CREATE TABLE IF NOT EXISTS application_state_deltas (
    version      bigint PRIMARY KEY,
    axis         text NOT NULL,
    entity_id    text NOT NULL,
    payload_json text NOT NULL,
    at_ms        bigint NOT NULL
);
CREATE INDEX IF NOT EXISTS application_state_deltas_axis_idx
    ON application_state_deltas(axis, version);
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

fn map_application_operation(r: &Row) -> ApplicationOperationRecord {
    ApplicationOperationRecord {
        logical_store_id: r.get("logical_store_id"),
        application_responsibility: r.get("application_responsibility"),
        operation_id: r.get("operation_id"),
        operation_kind: r.get("operation_kind"),
        sender: r.get("sender"),
        recipients_json: r.get("recipients_json"),
        payload_fingerprint: r.get("payload_fingerprint"),
        retry_budget: r.get("retry_budget"),
        state: r.get("state"),
        result_json: r.get("result_json"),
        recovery_json: r.get("recovery_json"),
        created_at_ms: r.get("created_at_ms"),
        updated_at_ms: r.get("updated_at_ms"),
        completed_at_ms: r.get("completed_at_ms"),
    }
}

fn map_compound_step(r: &Row) -> CompoundStepRecord {
    CompoundStepRecord {
        logical_store_id: r.get("logical_store_id"),
        application_responsibility: r.get("application_responsibility"),
        operation_id: r.get("operation_id"),
        step_id: r.get("step_id"),
        position: r.get("position"),
        step_kind: r.get("step_kind"),
        prerequisites_json: r.get("prerequisites_json"),
        declaration_json: r.get("declaration_json"),
        state: r.get("state"),
        outcome_json: r.get("outcome_json"),
        recovery_json: r.get("recovery_json"),
        created_at_ms: r.get("created_at_ms"),
        updated_at_ms: r.get("updated_at_ms"),
        completed_at_ms: r.get("completed_at_ms"),
    }
}

async fn validate_compound_prerequisites_postgres(
    tx: &Transaction<'_>,
    step: &CompoundDispositionStep,
) -> Result<()> {
    let prerequisites_json: String = tx
        .query_opt(
            "SELECT prerequisites_json FROM application_compound_steps
             WHERE logical_store_id=$1 AND application_responsibility=$2
               AND operation_id=$3 AND step_id=$4 FOR UPDATE",
            &[
                &step.logical_store_id,
                &step.application_responsibility,
                &step.operation_id,
                &step.step_id,
            ],
        )
        .await?
        .ok_or_else(|| anyhow!("compound step does not exist"))?
        .get("prerequisites_json");
    let prerequisites: Vec<String> = serde_json::from_str(&prerequisites_json)?;
    for prerequisite in prerequisites {
        let prerequisite_state = tx
            .query_opt(
                "SELECT state FROM application_compound_steps
                 WHERE logical_store_id=$1 AND application_responsibility=$2
                   AND operation_id=$3 AND step_id=$4 FOR UPDATE",
                &[
                    &step.logical_store_id,
                    &step.application_responsibility,
                    &step.operation_id,
                    &prerequisite,
                ],
            )
            .await?
            .map(|row| row.get::<_, String>("state"));
        if !matches!(
            prerequisite_state.as_deref(),
            Some("accepted" | "completed" | "no-op")
        ) {
            bail!("compound prerequisite is not durably complete");
        }
    }
    Ok(())
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

async fn pg_advance_clock_hwm(client: &tokio_postgres::Client) -> Result<i64> {
    let now = pg_now_ms(client).await?;
    Ok(client
        .query_one(
            "INSERT INTO clock_hwm(id, hwm_ms) VALUES (1, $1)
             ON CONFLICT(id) DO UPDATE
             SET hwm_ms = GREATEST(clock_hwm.hwm_ms + 1, EXCLUDED.hwm_ms)
             RETURNING hwm_ms",
            &[&now],
        )
        .await?
        .get(0))
}

async fn pg_tx_advance_clock_hwm(tx: &Transaction<'_>) -> Result<i64> {
    let now = pg_tx_now_ms(tx).await?;
    Ok(tx
        .query_one(
            "INSERT INTO clock_hwm(id, hwm_ms) VALUES (1, $1)
             ON CONFLICT(id) DO UPDATE
             SET hwm_ms = GREATEST(clock_hwm.hwm_ms + 1, EXCLUDED.hwm_ms)
             RETURNING hwm_ms",
            &[&now],
        )
        .await?
        .get(0))
}

async fn pg_tx_append_state_delta(
    tx: &Transaction<'_>,
    axis: &str,
    entity_id: &str,
    payload_json: &str,
) -> Result<StateDeltaRecord> {
    let version: i64 = tx
        .query_one(
            "UPDATE application_state_version
             SET version=version + 1
             WHERE singleton=1
             RETURNING version",
            &[],
        )
        .await?
        .get("version");
    let at_ms = pg_tx_advance_clock_hwm(tx).await?;
    tx.execute(
        "INSERT INTO application_state_deltas(version, axis, entity_id, payload_json, at_ms)
         VALUES ($1,$2,$3,$4,$5)",
        &[&version, &axis, &entity_id, &payload_json, &at_ms],
    )
    .await?;
    Ok(StateDeltaRecord {
        version,
        axis: axis.to_string(),
        entity_id: entity_id.to_string(),
        payload_json: payload_json.to_string(),
        at_ms,
    })
}

async fn pg_insert_message(
    tx: &Transaction<'_>,
    m: &NewMessage,
    operation: Option<&ApplicationMessageOperation>,
) -> Result<MessageRow> {
    if let Some(operation) = operation {
        let operation_row = tx
            .query_opt(
                "SELECT sender, payload_fingerprint, state FROM application_operations
                 WHERE logical_store_id=$1 AND application_responsibility=$2
                   AND operation_id=$3 FOR UPDATE",
                &[
                    &operation.logical_store_id,
                    &operation.application_responsibility,
                    &operation.operation_id,
                ],
            )
            .await?;
        let Some(operation_row) = operation_row else {
            bail!("application operation does not exist");
        };
        let sender: String = operation_row.get("sender");
        let fingerprint: String = operation_row.get("payload_fingerprint");
        let state: String = operation_row.get("state");
        if sender != m.from_addr.as_deref().unwrap_or_default()
            || fingerprint != operation.payload_fingerprint
            || !matches!(state.as_str(), "pending" | "needs-attach" | "indeterminate")
        {
            bail!("application operation evidence does not match message authorship");
        }
        if let Some(existing) = tx
            .query_opt(
                &format!(
                    "SELECT {MSG_COLS_M}
                     FROM application_operation_messages aom
                     JOIN messages m ON m.id=aom.message_id
                     WHERE aom.logical_store_id=$1
                       AND aom.application_responsibility=$2
                       AND aom.operation_id=$3"
                ),
                &[
                    &operation.logical_store_id,
                    &operation.application_responsibility,
                    &operation.operation_id,
                ],
            )
            .await?
        {
            return Ok(map_message(&existing));
        }
    }
    let now = pg_tx_advance_clock_hwm(tx).await?;
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
            "INSERT INTO messages(thread_id, parent_id, from_addr, to_addr, cc, kind, attention,
                 requires_disposition, subject, body, metadata, sent_at_ms, created_at_ms)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) RETURNING id",
            &[
                &parent_thread,
                &m.parent_id,
                &m.from_addr,
                &m.to_addr,
                &m.cc,
                &m.kind,
                &m.attention.as_str(),
                &m.requires_disposition,
                &m.subject,
                &m.body,
                &m.metadata,
                &m.sent_at_ms,
                &now,
            ],
        )
        .await?
        .get("id");
    if let Some(operation) = operation {
        tx.execute(
            "INSERT INTO application_operation_messages(
                 logical_store_id, application_responsibility, operation_id, message_id
             ) VALUES ($1,$2,$3,$4)",
            &[
                &operation.logical_store_id,
                &operation.application_responsibility,
                &operation.operation_id,
                &id,
            ],
        )
        .await?;
    }
    if parent_thread.is_none() {
        tx.execute("UPDATE messages SET thread_id=$1 WHERE id=$1", &[&id])
            .await?;
    }
    for recipient in fanout_recipients(&m.to_addr, m.cc.as_deref()) {
        let consumed_at_ms = (recipient != m.to_addr).then_some(now);
        tx.execute(
            "INSERT INTO deliveries(message_id, recipient, delivered_at_ms, consumed_at_ms)
             VALUES ($1,$2,$3,$4)
             ON CONFLICT(message_id, recipient) DO NOTHING",
            &[&id, &recipient, &now, &consumed_at_ms],
        )
        .await?;
        if consumed_at_ms.is_some() {
            let delivery_id: i64 = tx
                .query_one(
                    "SELECT id FROM deliveries WHERE message_id=$1 AND recipient=$2",
                    &[&id, &recipient],
                )
                .await?
                .get("id");
            pg_tx_append_state_delta(
                tx,
                "acknowledgment",
                &format!("delivery:{delivery_id}"),
                &serde_json::json!({
                    "delivery_id": delivery_id,
                    "message_id": id,
                    "recipient": recipient,
                })
                .to_string(),
            )
            .await?;
        }
    }
    pg_tx_append_state_delta(
        tx,
        "message",
        &format!("message:{id}"),
        &serde_json::json!({
            "message_id": id,
            "thread_id": parent_thread.unwrap_or(id),
            "to_addr": m.to_addr,
            "attention": m.attention.as_str(),
            "kind": m.kind,
        })
        .to_string(),
    )
    .await?;
    Ok(map_message(
        &tx.query_one(
            &format!("SELECT {MSG_COLS} FROM messages WHERE id=$1"),
            &[&id],
        )
        .await?,
    ))
}

async fn raise_clock_hwm_to_existing_timestamps(client: &tokio_postgres::Client) -> Result<()> {
    client
        .execute(
            "UPDATE clock_hwm
             SET hwm_ms = GREATEST(
                hwm_ms,
                COALESCE((SELECT MAX(created_at_ms) FROM messages), 0),
                COALESCE((SELECT MAX(sent_at_ms) FROM messages), 0),
                COALESCE((SELECT MAX(delivered_at_ms) FROM deliveries), 0),
                COALESCE((SELECT MAX(consumed_at_ms) FROM deliveries), 0),
                COALESCE((SELECT MAX(at_ms) FROM dispositions), 0)
             )
             WHERE id = 1",
            &[],
        )
        .await?;
    Ok(())
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
             WHERE (m.to_addr=$1 OR EXISTS (
                   SELECT 1 FROM unnest(string_to_array(COALESCE(m.cc, ''), ',')) AS cc_addr
                   WHERE btrim(cc_addr) = $1
             ))
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
         WHERE (m.to_addr=$1 OR EXISTS (
               SELECT 1 FROM unnest(string_to_array(COALESCE(m.cc, ''), ',')) AS cc_addr
               WHERE btrim(cc_addr) = $1
         ))
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

async fn current_schema_version(client: &tokio_postgres::Client) -> Result<i64> {
    let exists: bool = client
        .query_one(
            "SELECT to_regclass('telex_schema_version') IS NOT NULL",
            &[],
        )
        .await?
        .get(0);
    if !exists {
        return Ok(0);
    }
    Ok(client
        .query_one(
            "SELECT COALESCE(MAX(version), 0) FROM telex_schema_version",
            &[],
        )
        .await?
        .get(0))
}

async fn publish_schema_version(client: &tokio_postgres::Client) -> Result<()> {
    client
        .execute(
            "INSERT INTO telex_schema_version(singleton, version) VALUES (1, $1)
             ON CONFLICT(singleton) DO UPDATE SET version = GREATEST(telex_schema_version.version, $1)",
            &[&CURRENT_SCHEMA_VERSION],
        )
        .await?;
    Ok(())
}

impl PgBackend {
    /// Connect using a fully-built config (host/user/db/password) and an optional schema
    /// to isolate telex tables in. The password is resolved by the caller (profile).
    pub async fn connect_with(
        config: tokio_postgres::Config,
        schema: Option<&str>,
    ) -> Result<Self> {
        let client = Self::open_client(&config, schema).await?;
        let notify_channel = notify_channel_for_schema(schema)?;
        Ok(Self {
            connector: PgConnector::Static {
                config,
                schema: schema.map(str::to_string),
            },
            client: AsyncMutex::new(client),
            notify_channel,
        })
    }

    pub async fn connect_profile(profile: crate::profiles::BackendProfile) -> Result<Self> {
        let (config, schema) = crate::profiles::pg_connect_config(&profile).await?;
        let client = Self::open_client(&config, schema.as_deref()).await?;
        let notify_channel = notify_channel_for_schema(schema.as_deref())?;
        Ok(Self {
            connector: PgConnector::Profile(profile),
            client: AsyncMutex::new(client),
            notify_channel,
        })
    }

    async fn open_client(
        config: &tokio_postgres::Config,
        schema: Option<&str>,
    ) -> Result<tokio_postgres::Client> {
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
        Ok(client)
    }

    async fn client(&self) -> Result<MutexGuard<'_, tokio_postgres::Client>> {
        let mut client = self.client.lock().await;
        if client.is_closed() {
            *client = self.reconnect_client().await?;
        }
        Ok(client)
    }

    async fn reconnect_client(&self) -> Result<tokio_postgres::Client> {
        match &self.connector {
            PgConnector::Static { config, schema } => {
                Self::open_client(config, schema.as_deref()).await
            }
            PgConnector::Profile(profile) => {
                let (config, schema) = crate::profiles::pg_connect_config(profile)
                    .await
                    .context("resolving postgres reconnect profile")?;
                Self::open_client(&config, schema.as_deref()).await
            }
        }
        .context("reconnecting to postgres")
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

    fn supports_wake_on_cc(&self) -> bool {
        true
    }

    async fn init_schema(&self) -> Result<()> {
        let mut client = self.client().await?;
        let schema_version = current_schema_version(&client).await?;
        if schema_version > CURRENT_SCHEMA_VERSION {
            bail!(
                "Postgres schema version {schema_version} is newer than supported version {CURRENT_SCHEMA_VERSION}"
            );
        }
        client.batch_execute(SCHEMA).await?;
        let store_id = new_logical_store_id()?;
        client
            .execute(
                "INSERT INTO application_store_identity(singleton, logical_store_id)
                 VALUES (1, $1) ON CONFLICT(singleton) DO NOTHING",
                &[&store_id],
            )
            .await?;
        client
            .batch_execute(
                "ALTER TABLE leases ADD COLUMN IF NOT EXISTS lease_epoch bigint;
                 ALTER TABLE leases ADD COLUMN IF NOT EXISTS owner_instance_id text;
                 ALTER TABLE deliveries ADD COLUMN IF NOT EXISTS consumed_at_ms bigint;
                 CREATE TABLE IF NOT EXISTS clock_hwm (
                    id     integer PRIMARY KEY CHECK (id = 1),
                    hwm_ms bigint NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS deliveries_recipient_pending_idx
                    ON deliveries(recipient, consumed_at_ms, message_id);
                 CREATE TABLE IF NOT EXISTS telex_schema_meta (
                    key   text PRIMARY KEY,
                    value text NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS telex_schema_version (
                    singleton integer NOT NULL DEFAULT 1 UNIQUE,
                    version   bigint NOT NULL
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
        let now = pg_now_ms(&client).await?;
        client
            .execute(
                "INSERT INTO clock_hwm(id, hwm_ms) VALUES (1, $1)
                 ON CONFLICT(id) DO NOTHING",
                &[&now],
            )
            .await?;
        backfill_existing_deliveries_consumed_once(&mut client).await?;
        raise_clock_hwm_to_existing_timestamps(&client).await?;
        publish_schema_version(&client).await?;
        Ok(())
    }

    async fn logical_store_id(&self) -> Result<String> {
        let client = self.client().await?;
        Ok(client
            .query_one(
                "SELECT logical_store_id FROM application_store_identity WHERE singleton=1",
                &[],
            )
            .await?
            .get("logical_store_id"))
    }

    async fn ensure_address(
        &self,
        address: &str,
        description: Option<&str>,
        scope: Option<&str>,
        tags: Option<&str>,
    ) -> Result<()> {
        let client = self.client().await?;
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
        let client = self.client().await?;
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
        let client = self.client().await?;
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
            let client = self.client().await?;
            client.query(&sql, &[&s]).await?
        } else {
            sql.push_str(" ORDER BY address");
            let client = self.client().await?;
            client.query(&sql, &[]).await?
        };
        Ok(rows.iter().map(map_address).collect())
    }

    async fn claim_lease(&self, claim: &LeaseClaim, window_secs: i64) -> Result<LeaseOutcome> {
        let client = self.client().await?;
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
        let client = self.client().await?;
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
        let client = self.client().await?;
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
        let client = self.client().await?;
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
        let client = self.client().await?;
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
        let mut client = self.client().await?;
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
                let Some(lease_epoch) = lease_epoch else {
                    unreachable!("lease_epoch is Some in the non-legacy branch");
                };
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
        let client = self.client().await?;
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
        let client = self.client().await?;
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
        let mut client = self.client().await?;
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
        let client = self.client().await?;
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
        let mut client = self.client().await?;
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
                        let delivery_id: i64 = tx
                            .query_one(
                                "SELECT id FROM deliveries
                                 WHERE message_id=$1 AND recipient=$2",
                                &[&message_id, &recipient],
                            )
                            .await?
                            .get("id");
                        pg_tx_append_state_delta(
                            &tx,
                            "acknowledgment",
                            &format!("delivery:{delivery_id}"),
                            &serde_json::json!({
                                "delivery_id": delivery_id,
                                "message_id": message_id,
                                "recipient": recipient,
                            })
                            .to_string(),
                        )
                        .await?;
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

    async fn mark_delivery_consumed_if_current_owner(
        &self,
        recipient: &str,
        owner_instance_id: &str,
        lease_epoch: i64,
        message_id: i64,
        delivery_id: i64,
    ) -> Result<DeliveryOutcome> {
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        let lease = tx
            .query_opt(
                "SELECT lease_epoch, owner_instance_id
                 FROM leases WHERE address=$1 FOR UPDATE",
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
        let row = tx
            .query_opt(
                "SELECT id, consumed_at_ms
                 FROM deliveries
                 WHERE message_id=$1 AND recipient=$2
                 FOR UPDATE",
                &[&message_id, &recipient],
            )
            .await?;
        let outcome = match row {
            None => DeliveryOutcome::AckNoOp,
            Some(row) => {
                let actual_id: i64 = row.get("id");
                let consumed_at_ms: Option<i64> = row.get("consumed_at_ms");
                if actual_id != delivery_id {
                    DeliveryOutcome::DeliveryMismatch
                } else if consumed_at_ms.is_some() {
                    DeliveryOutcome::AlreadyConsumed
                } else {
                    let now = pg_tx_now_ms(&tx).await?;
                    let updated = tx
                        .execute(
                            "UPDATE deliveries SET consumed_at_ms=$1
                             WHERE id=$2 AND message_id=$3 AND recipient=$4
                               AND consumed_at_ms IS NULL",
                            &[&now, &delivery_id, &message_id, &recipient],
                        )
                        .await?;
                    if updated == 0 {
                        DeliveryOutcome::AlreadyConsumed
                    } else {
                        pg_tx_append_state_delta(
                            &tx,
                            "acknowledgment",
                            &format!("delivery:{delivery_id}"),
                            &format!(
                                "{{\"delivery_id\":{delivery_id},\"message_id\":{message_id},\"recipient\":{}}}",
                                serde_json::to_string(recipient)?
                            ),
                        )
                        .await?;
                        DeliveryOutcome::Marked
                    }
                }
            }
        };
        tx.commit().await?;
        Ok(outcome)
    }

    async fn durable_clock_now_ms(&self) -> Result<i64> {
        let client = self.client().await?;
        pg_advance_clock_hwm(&client).await
    }

    async fn delivery_retention_count(&self) -> Result<i64> {
        let client = self.client().await?;
        Ok(client
            .query_one("SELECT COUNT(*) FROM deliveries", &[])
            .await?
            .get(0))
    }

    async fn pending_unconsumed_count(&self, address: &str) -> Result<i64> {
        let client = self.client().await?;
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

    async fn inbound_actionable_count(&self, address: &str) -> Result<i64> {
        let client = self.client().await?;
        materialize_pending_delivery_rows_for_recipient(&client, address).await?;
        // Actionable inbound = requires this recipient's disposition (primary to_addr +
        // requires_disposition), not consumed, not terminally dispositioned.
        let sql = format!(
            "SELECT COUNT(*) FROM deliveries d
             JOIN messages m ON m.id=d.message_id
             WHERE d.recipient=$1
               AND d.consumed_at_ms IS NULL
               AND m.requires_disposition = TRUE
               AND m.to_addr=$1
               AND COALESCE((SELECT disp.state FROM dispositions disp
                              WHERE disp.message_id=m.id AND disp.recipient=$1
                              ORDER BY disp.id DESC LIMIT 1), '') NOT IN ({})",
            terminal_dispositions_sql_list()
        );
        Ok(client.query_one(&sql, &[&address]).await?.get(0))
    }

    async fn pending_and_actionable_counts(&self, address: &str) -> Result<(i64, i64)> {
        let client = self.client().await?;
        // Materialize once, then run both counts on the same connection.
        materialize_pending_delivery_rows_for_recipient(&client, address).await?;
        let terminal = terminal_dispositions_sql_list();
        let pending_sql = format!(
            "SELECT COUNT(*) FROM deliveries d
             JOIN messages m ON m.id=d.message_id
             WHERE d.recipient=$1
               AND d.consumed_at_ms IS NULL
               AND COALESCE((SELECT disp.state FROM dispositions disp
                              WHERE disp.message_id=m.id AND disp.recipient=$1
                              ORDER BY disp.id DESC LIMIT 1), '') NOT IN ({terminal})"
        );
        let actionable_sql = format!(
            "SELECT COUNT(*) FROM deliveries d
             JOIN messages m ON m.id=d.message_id
             WHERE d.recipient=$1
               AND d.consumed_at_ms IS NULL
               AND m.requires_disposition = TRUE
               AND m.to_addr=$1
               AND COALESCE((SELECT disp.state FROM dispositions disp
                              WHERE disp.message_id=m.id AND disp.recipient=$1
                              ORDER BY disp.id DESC LIMIT 1), '') NOT IN ({terminal})"
        );
        let pending: i64 = client.query_one(&pending_sql, &[&address]).await?.get(0);
        let actionable: i64 = client.query_one(&actionable_sql, &[&address]).await?.get(0);
        Ok((pending, actionable))
    }

    async fn record_detach_tombstone(
        &self,
        session_id: &str,
        address: &str,
        reason: &str,
    ) -> Result<()> {
        let client = self.client().await?;
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
        let client = self.client().await?;
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
        let client = self.client().await?;
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
        let client = self.client().await?;
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
        let client = self.client().await?;
        materialize_pending_delivery_rows_for_recipient(&client, address).await?;
        let sql = format!(
            "SELECT {MSG_COLS_M}, d.id AS delivery_id,
                    (SELECT version FROM application_state_version WHERE singleton=1)
                    AS snapshot_version
             FROM deliveries d
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

    async fn fetch_wait_candidates(
        &self,
        address: &str,
        options: WaitFetchOptions,
    ) -> Result<Vec<WaitCandidate>> {
        let client = self.client().await?;
        materialize_pending_delivery_rows_for_recipient(&client, address).await?;
        let terminal = terminal_dispositions_sql_list();
        let primary_sql = format!(
            "SELECT {MSG_COLS_M}, d.id AS delivery_id FROM deliveries d
             JOIN messages m ON m.id=d.message_id
             WHERE d.recipient=$1
               AND d.consumed_at_ms IS NULL
               AND COALESCE((SELECT disp.state FROM dispositions disp
                             WHERE disp.message_id=m.id AND disp.recipient=$1
                             ORDER BY disp.id DESC LIMIT 1), '') NOT IN ({terminal})
             ORDER BY d.message_id"
        );
        let mut candidates: Vec<WaitCandidate> = client
            .query(&primary_sql, &[&address])
            .await?
            .into_iter()
            .map(|row| {
                WaitCandidate::primary(
                    map_message(&row),
                    Some(row.get("delivery_id")),
                    row.get("snapshot_version"),
                )
            })
            .collect();

        if options.wake_on_cc {
            let cc_sql = format!(
                "SELECT {MSG_COLS_M}, d.id AS delivery_id,
                        (SELECT version FROM application_state_version WHERE singleton=1)
                        AS snapshot_version
                 FROM deliveries d
                 JOIN messages m ON m.id=d.message_id
                 WHERE d.recipient=$1
                   AND d.consumed_at_ms IS NOT NULL
                   AND d.delivered_at_ms > $2
                   AND COALESCE((SELECT disp.state FROM dispositions disp
                                 WHERE disp.message_id=m.id AND disp.recipient=$1
                                 ORDER BY disp.id DESC LIMIT 1), '') NOT IN ({terminal})
                 ORDER BY d.message_id"
            );
            let cc_messages = client
                .query(&cc_sql, &[&address, &options.cc_after_ms])
                .await?;
            candidates.extend(cc_messages.into_iter().filter_map(|row| {
                let message = map_message(&row);
                let delivery_id = row.get("delivery_id");
                let snapshot_version = row.get("snapshot_version");
                (delivery_role(address, &message.to_addr, message.cc.as_deref()) == "cc").then(
                    || WaitCandidate::cc_notification(message, Some(delivery_id), snapshot_version),
                )
            }));
        }

        candidates.sort_by_key(|candidate| candidate.message.id);
        Ok(candidates)
    }

    async fn has_delivery_for_recipient(&self, message_id: i64, recipient: &str) -> Result<bool> {
        let client = self.client().await?;
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
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        let message = pg_insert_message(&tx, m, None).await?;
        tx.commit().await?;
        Ok(message)
    }

    async fn insert_application_message(
        &self,
        message: &NewMessage,
        operation: &ApplicationMessageOperation,
    ) -> Result<MessageRow> {
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        let message = pg_insert_message(&tx, message, Some(operation)).await?;
        tx.commit().await?;
        Ok(message)
    }

    async fn get_message(&self, id: i64) -> Result<Option<MessageRow>> {
        let client = self.client().await?;
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
        let client = self.client().await?;
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
        let client = self.client().await?;
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
            let client = self.client().await?;
            client.query(&sql, &[&since, &addr]).await?
        } else {
            let client = self.client().await?;
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
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        let now = pg_tx_advance_clock_hwm(&tx).await?;
        let id: i64 = tx
            .query_one(
                "INSERT INTO dispositions(message_id, recipient, state, note, by_principal, at_ms) \
                 VALUES ($1,$2,$3,$4,$5,$6) RETURNING id",
                &[&message_id, &recipient, &state, &note, &by, &now],
            )
            .await?
            .get("id");
        if Disposition::is_terminal_str(state) {
            materialize_pending_delivery_rows_for_recipient_tx(&tx, recipient).await?;
            let consumed = tx
                .execute(
                    "UPDATE deliveries SET consumed_at_ms=$1
                 WHERE message_id=$2 AND recipient=$3 AND consumed_at_ms IS NULL",
                    &[&now, &message_id, &recipient],
                )
                .await?;
            if consumed > 0 {
                let delivery_id: i64 = tx
                    .query_one(
                        "SELECT id FROM deliveries
                         WHERE message_id=$1 AND recipient=$2",
                        &[&message_id, &recipient],
                    )
                    .await?
                    .get("id");
                pg_tx_append_state_delta(
                    &tx,
                    "acknowledgment",
                    &format!("delivery:{delivery_id}"),
                    &serde_json::json!({
                        "delivery_id": delivery_id,
                        "message_id": message_id,
                        "recipient": recipient,
                    })
                    .to_string(),
                )
                .await?;
            }
        }
        pg_tx_append_state_delta(
            &tx,
            "disposition",
            &format!("message:{message_id}:recipient:{recipient}"),
            &serde_json::json!({
                "message_id": message_id,
                "recipient": recipient,
                "state": state,
                "is_terminal": Disposition::is_terminal_str(state),
            })
            .to_string(),
        )
        .await?;
        let row = DispositionRow {
            id,
            message_id,
            recipient: recipient.to_string(),
            state: state.to_string(),
            note: note.map(str::to_string),
            by_principal: by.map(str::to_string),
            at_ms: now,
        };
        tx.commit().await?;
        Ok(row)
    }

    #[allow(clippy::too_many_arguments)]
    async fn application_disposition_with_ack(
        &self,
        recipient: &str,
        owner_instance_id: &str,
        lease_epoch: i64,
        message_id: i64,
        delivery_id: i64,
        state: &str,
        note: Option<&str>,
        by: Option<&str>,
        compound_step: Option<&CompoundDispositionStep>,
    ) -> Result<(Option<DispositionRow>, DeliveryOutcome)> {
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        let lease = tx
            .query_opt(
                "SELECT lease_epoch, owner_instance_id FROM leases
                 WHERE address=$1 FOR UPDATE",
                &[&recipient],
            )
            .await?;
        let is_owner = lease.is_some_and(|row| {
            row.get::<_, Option<i64>>("lease_epoch") == Some(lease_epoch)
                && row.get::<_, Option<String>>("owner_instance_id").as_deref()
                    == Some(owner_instance_id)
        });
        if !is_owner {
            tx.rollback().await?;
            return Ok((None, DeliveryOutcome::NotOwner));
        }
        materialize_pending_delivery_rows_for_recipient_tx(&tx, recipient).await?;
        let delivery = tx
            .query_opt(
                "SELECT id, consumed_at_ms FROM deliveries
                 WHERE message_id=$1 AND recipient=$2 FOR UPDATE",
                &[&message_id, &recipient],
            )
            .await?;
        let Some(delivery) = delivery else {
            tx.rollback().await?;
            return Ok((None, DeliveryOutcome::AckNoOp));
        };
        if delivery.get::<_, i64>("id") != delivery_id {
            tx.rollback().await?;
            return Ok((None, DeliveryOutcome::DeliveryMismatch));
        }
        if let Some(compound) = compound_step {
            validate_compound_prerequisites_postgres(&tx, compound).await?;
        }
        let consumed_at_ms: Option<i64> = delivery.get("consumed_at_ms");
        let now = pg_tx_advance_clock_hwm(&tx).await?;
        let id: i64 = tx
            .query_one(
                "INSERT INTO dispositions(message_id, recipient, state, note, by_principal, at_ms)
                 VALUES ($1,$2,$3,$4,$5,$6) RETURNING id",
                &[&message_id, &recipient, &state, &note, &by, &now],
            )
            .await?
            .get("id");
        let mut outcome = if consumed_at_ms.is_some() {
            DeliveryOutcome::AlreadyConsumed
        } else {
            DeliveryOutcome::AckNoOp
        };
        if Disposition::is_terminal_str(state) && consumed_at_ms.is_none() {
            tx.execute(
                "UPDATE deliveries SET consumed_at_ms=$1
                 WHERE id=$2 AND message_id=$3 AND recipient=$4",
                &[&now, &delivery_id, &message_id, &recipient],
            )
            .await?;
            pg_tx_append_state_delta(
                &tx,
                "acknowledgment",
                &format!("delivery:{delivery_id}"),
                &serde_json::json!({
                    "delivery_id": delivery_id,
                    "message_id": message_id,
                    "recipient": recipient,
                })
                .to_string(),
            )
            .await?;
            if let Some(compound) = compound_step {
                let changed = tx
                    .execute(
                        "UPDATE application_compound_steps
                         SET state='accepted', outcome_json=$5, recovery_json=$6,
                             updated_at_ms=$7, completed_at_ms=$7
                         WHERE logical_store_id=$1 AND application_responsibility=$2
                           AND operation_id=$3 AND step_id=$4",
                        &[
                            &compound.logical_store_id,
                            &compound.application_responsibility,
                            &compound.operation_id,
                            &compound.step_id,
                            &compound.outcome_json,
                            &compound.recovery_json,
                            &now,
                        ],
                    )
                    .await?;
                if changed == 0 {
                    bail!("compound step does not exist");
                }
                pg_tx_append_state_delta(
                    &tx,
                    "compound",
                    &format!(
                        "operation:{}:step:{}",
                        compound.operation_id, compound.step_id
                    ),
                    &serde_json::json!({
                        "operation_id": compound.operation_id,
                        "step_id": compound.step_id,
                        "state": "accepted",
                    })
                    .to_string(),
                )
                .await?;
            }
            outcome = DeliveryOutcome::Marked;
        }
        pg_tx_append_state_delta(
            &tx,
            "disposition",
            &format!("message:{message_id}:recipient:{recipient}"),
            &serde_json::json!({
                "message_id": message_id,
                "recipient": recipient,
                "state": state,
                "is_terminal": Disposition::is_terminal_str(state),
            })
            .to_string(),
        )
        .await?;
        tx.commit().await?;
        Ok((
            Some(DispositionRow {
                id,
                message_id,
                recipient: recipient.to_string(),
                state: state.to_string(),
                note: note.map(str::to_string),
                by_principal: by.map(str::to_string),
                at_ms: now,
            }),
            outcome,
        ))
    }

    async fn dispositions_for(&self, message_id: i64) -> Result<Vec<DispositionRow>> {
        let client = self.client().await?;
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

    async fn max_message_id(&self) -> Result<i64> {
        let client = self.client.lock().await;
        let row = client
            .query_one("SELECT COALESCE(MAX(id),0) AS m FROM messages", &[])
            .await?;
        Ok(row.get("m"))
    }

    async fn deliveries_for(&self, message_id: i64) -> Result<Vec<DeliveryRow>> {
        let client = self.client.lock().await;
        let rows = client
            .query(
                "SELECT id, message_id, recipient, occupant, delivered_at_ms, consumed_at_ms \
                 FROM deliveries WHERE message_id=$1 ORDER BY id",
                &[&message_id],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| DeliveryRow {
                id: r.get("id"),
                message_id: r.get("message_id"),
                recipient: r.get("recipient"),
                occupant: r.get("occupant"),
                delivered_at_ms: r.get("delivered_at_ms"),
                consumed_at_ms: r.get("consumed_at_ms"),
            })
            .collect())
    }

    async fn delivery_for_recipient(
        &self,
        message_id: i64,
        recipient: &str,
    ) -> Result<Option<DeliveryRow>> {
        let client = self.client().await?;
        materialize_pending_delivery_rows_for_recipient(&client, recipient).await?;
        Ok(client
            .query_opt(
                "SELECT id, message_id, recipient, occupant, delivered_at_ms, consumed_at_ms
                 FROM deliveries WHERE message_id=$1 AND recipient=$2",
                &[&message_id, &recipient],
            )
            .await?
            .map(|r| DeliveryRow {
                id: r.get("id"),
                message_id: r.get("message_id"),
                recipient: r.get("recipient"),
                occupant: r.get("occupant"),
                delivered_at_ms: r.get("delivered_at_ms"),
                consumed_at_ms: r.get("consumed_at_ms"),
            }))
    }

    async fn begin_application_operation(
        &self,
        operation: &NewApplicationOperation,
    ) -> Result<ApplicationOperationBegin> {
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        let existing = tx
            .query_opt(
                "SELECT logical_store_id, application_responsibility, operation_id,
                        operation_kind, sender, recipients_json, payload_fingerprint,
                        retry_budget, state, result_json, recovery_json, created_at_ms,
                        updated_at_ms, completed_at_ms
                 FROM application_operations
                 WHERE logical_store_id=$1 AND application_responsibility=$2
                   AND operation_id=$3
                 FOR UPDATE",
                &[
                    &operation.logical_store_id,
                    &operation.application_responsibility,
                    &operation.operation_id,
                ],
            )
            .await?;
        if let Some(existing) = existing {
            let existing = map_application_operation(&existing);
            tx.commit().await?;
            return if existing.payload_fingerprint == operation.payload_fingerprint {
                Ok(ApplicationOperationBegin::Replay(existing))
            } else {
                Ok(ApplicationOperationBegin::FingerprintMismatch(existing))
            };
        }

        tx.execute(
            "INSERT INTO application_operations(
                 logical_store_id, application_responsibility, operation_id,
                 operation_kind, sender, recipients_json, payload_fingerprint,
                 retry_budget, state, created_at_ms, updated_at_ms
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'pending',$9,$9)",
            &[
                &operation.logical_store_id,
                &operation.application_responsibility,
                &operation.operation_id,
                &operation.operation_kind,
                &operation.sender,
                &operation.recipients_json,
                &operation.payload_fingerprint,
                &operation.retry_budget,
                &operation.created_at_ms,
            ],
        )
        .await?;
        pg_tx_append_state_delta(
            &tx,
            "operation",
            &format!("operation:{}", operation.operation_id),
            &format!(
                "{{\"operation_id\":{},\"state\":\"pending\"}}",
                serde_json::to_string(&operation.operation_id)?
            ),
        )
        .await?;
        let inserted = tx
            .query_one(
                "SELECT logical_store_id, application_responsibility, operation_id,
                        operation_kind, sender, recipients_json, payload_fingerprint,
                        retry_budget, state, result_json, recovery_json, created_at_ms,
                        updated_at_ms, completed_at_ms
                 FROM application_operations
                 WHERE logical_store_id=$1 AND application_responsibility=$2
                   AND operation_id=$3",
                &[
                    &operation.logical_store_id,
                    &operation.application_responsibility,
                    &operation.operation_id,
                ],
            )
            .await?;
        let inserted = map_application_operation(&inserted);
        tx.commit().await?;
        Ok(ApplicationOperationBegin::Started(inserted))
    }

    async fn application_operation(
        &self,
        logical_store_id: &str,
        application_responsibility: &str,
        operation_id: &str,
    ) -> Result<Option<ApplicationOperationRecord>> {
        let client = self.client().await?;
        Ok(client
            .query_opt(
                "SELECT logical_store_id, application_responsibility, operation_id,
                        operation_kind, sender, recipients_json, payload_fingerprint,
                        retry_budget, state, result_json, recovery_json, created_at_ms,
                        updated_at_ms, completed_at_ms
                 FROM application_operations
                 WHERE logical_store_id=$1 AND application_responsibility=$2
                   AND operation_id=$3",
                &[
                    &logical_store_id,
                    &application_responsibility,
                    &operation_id,
                ],
            )
            .await?
            .map(|r| map_application_operation(&r)))
    }

    async fn application_operation_message(
        &self,
        logical_store_id: &str,
        application_responsibility: &str,
        operation_id: &str,
    ) -> Result<Option<MessageRow>> {
        let client = self.client().await?;
        Ok(client
            .query_opt(
                &format!(
                    "SELECT {MSG_COLS_M}
                     FROM application_operation_messages aom
                     JOIN messages m ON m.id=aom.message_id
                     WHERE aom.logical_store_id=$1
                       AND aom.application_responsibility=$2
                       AND aom.operation_id=$3"
                ),
                &[
                    &logical_store_id,
                    &application_responsibility,
                    &operation_id,
                ],
            )
            .await?
            .map(|row| map_message(&row)))
    }

    async fn complete_application_operation(
        &self,
        logical_store_id: &str,
        application_responsibility: &str,
        operation_id: &str,
        state: &str,
        result_json: Option<&str>,
        recovery_json: Option<&str>,
    ) -> Result<ApplicationOperationRecord> {
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        let now = pg_tx_advance_clock_hwm(&tx).await?;
        let row = tx
            .query_opt(
                "UPDATE application_operations
                 SET state=$4, result_json=$5, recovery_json=$6,
                     updated_at_ms=$7,
                     completed_at_ms=CASE
                         WHEN $4 IN ('accepted','rejected','duplicate','completed')
                         THEN $7 ELSE NULL END
                 WHERE logical_store_id=$1 AND application_responsibility=$2
                   AND operation_id=$3
                 RETURNING logical_store_id, application_responsibility, operation_id,
                           operation_kind, sender, recipients_json, payload_fingerprint,
                           retry_budget, state, result_json, recovery_json, created_at_ms,
                           updated_at_ms, completed_at_ms",
                &[
                    &logical_store_id,
                    &application_responsibility,
                    &operation_id,
                    &state,
                    &result_json,
                    &recovery_json,
                    &now,
                ],
            )
            .await?
            .ok_or_else(|| anyhow!("application operation does not exist"))?;
        pg_tx_append_state_delta(
            &tx,
            "operation",
            &format!("operation:{operation_id}"),
            &format!(
                "{{\"operation_id\":{},\"state\":{}}}",
                serde_json::to_string(operation_id)?,
                serde_json::to_string(state)?
            ),
        )
        .await?;
        let row = map_application_operation(&row);
        tx.commit().await?;
        Ok(row)
    }

    async fn declare_compound_steps(
        &self,
        steps: &[NewCompoundStepRecord],
    ) -> Result<Vec<CompoundStepRecord>> {
        if steps.is_empty() {
            return Ok(Vec::new());
        }
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        for step in steps {
            let existing = tx
                .query_opt(
                    "SELECT step_kind, prerequisites_json, declaration_json, position
                     FROM application_compound_steps
                     WHERE logical_store_id=$1 AND application_responsibility=$2
                       AND operation_id=$3 AND step_id=$4
                     FOR UPDATE",
                    &[
                        &step.logical_store_id,
                        &step.application_responsibility,
                        &step.operation_id,
                        &step.step_id,
                    ],
                )
                .await?;
            if let Some(existing) = existing {
                let existing_value = (
                    existing.get::<_, String>("step_kind"),
                    existing.get::<_, String>("prerequisites_json"),
                    existing.get::<_, String>("declaration_json"),
                    existing.get::<_, i64>("position"),
                );
                if existing_value
                    != (
                        step.step_kind.clone(),
                        step.prerequisites_json.clone(),
                        step.declaration_json.clone(),
                        step.position,
                    )
                {
                    bail!("compound step declaration mismatch for {}", step.step_id);
                }
                continue;
            }
            tx.execute(
                "INSERT INTO application_compound_steps(
                     logical_store_id, application_responsibility, operation_id,
                     step_id, position, step_kind, prerequisites_json,
                     declaration_json, state, created_at_ms, updated_at_ms
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'pending',$9,$9)",
                &[
                    &step.logical_store_id,
                    &step.application_responsibility,
                    &step.operation_id,
                    &step.step_id,
                    &step.position,
                    &step.step_kind,
                    &step.prerequisites_json,
                    &step.declaration_json,
                    &step.created_at_ms,
                ],
            )
            .await?;
            pg_tx_append_state_delta(
                &tx,
                "compound",
                &format!("operation:{}:step:{}", step.operation_id, step.step_id),
                &format!(
                    "{{\"operation_id\":{},\"step_id\":{},\"state\":\"pending\"}}",
                    serde_json::to_string(&step.operation_id)?,
                    serde_json::to_string(&step.step_id)?
                ),
            )
            .await?;
        }
        let first = &steps[0];
        let rows = tx
            .query(
                "SELECT logical_store_id, application_responsibility, operation_id,
                        step_id, position, step_kind, prerequisites_json, declaration_json,
                        state, outcome_json, recovery_json, created_at_ms, updated_at_ms,
                        completed_at_ms
                 FROM application_compound_steps
                 WHERE logical_store_id=$1 AND application_responsibility=$2
                   AND operation_id=$3
                 ORDER BY position, step_id",
                &[
                    &first.logical_store_id,
                    &first.application_responsibility,
                    &first.operation_id,
                ],
            )
            .await?;
        let records = rows.iter().map(map_compound_step).collect();
        tx.commit().await?;
        Ok(records)
    }

    async fn compound_steps(
        &self,
        logical_store_id: &str,
        application_responsibility: &str,
        operation_id: &str,
    ) -> Result<Vec<CompoundStepRecord>> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT logical_store_id, application_responsibility, operation_id,
                        step_id, position, step_kind, prerequisites_json, declaration_json,
                        state, outcome_json, recovery_json, created_at_ms, updated_at_ms,
                        completed_at_ms
                 FROM application_compound_steps
                 WHERE logical_store_id=$1 AND application_responsibility=$2
                   AND operation_id=$3
                 ORDER BY position, step_id",
                &[
                    &logical_store_id,
                    &application_responsibility,
                    &operation_id,
                ],
            )
            .await?;
        Ok(rows.iter().map(map_compound_step).collect())
    }

    #[allow(clippy::too_many_arguments)]
    async fn complete_compound_step(
        &self,
        logical_store_id: &str,
        application_responsibility: &str,
        operation_id: &str,
        step_id: &str,
        state: &str,
        outcome_json: Option<&str>,
        recovery_json: Option<&str>,
    ) -> Result<CompoundStepRecord> {
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        let prerequisites_json: String = tx
            .query_opt(
                "SELECT prerequisites_json FROM application_compound_steps
                 WHERE logical_store_id=$1 AND application_responsibility=$2
                   AND operation_id=$3 AND step_id=$4 FOR UPDATE",
                &[
                    &logical_store_id,
                    &application_responsibility,
                    &operation_id,
                    &step_id,
                ],
            )
            .await?
            .ok_or_else(|| anyhow!("compound step does not exist"))?
            .get("prerequisites_json");
        let prerequisites: Vec<String> = serde_json::from_str(&prerequisites_json)?;
        for prerequisite in prerequisites {
            let prerequisite_state = tx
                .query_opt(
                    "SELECT state FROM application_compound_steps
                     WHERE logical_store_id=$1 AND application_responsibility=$2
                       AND operation_id=$3 AND step_id=$4 FOR UPDATE",
                    &[
                        &logical_store_id,
                        &application_responsibility,
                        &operation_id,
                        &prerequisite,
                    ],
                )
                .await?
                .map(|row| row.get::<_, String>("state"));
            if !matches!(
                prerequisite_state.as_deref(),
                Some("accepted" | "completed" | "no-op")
            ) {
                bail!("compound prerequisite is not durably complete");
            }
        }
        let now = pg_tx_advance_clock_hwm(&tx).await?;
        let row = tx
            .query_opt(
                "UPDATE application_compound_steps
                 SET state=$5, outcome_json=$6, recovery_json=$7,
                     updated_at_ms=$8,
                     completed_at_ms=CASE
                         WHEN $5 IN ('accepted','rejected','completed','no-op')
                         THEN $8 ELSE NULL END
                 WHERE logical_store_id=$1 AND application_responsibility=$2
                   AND operation_id=$3 AND step_id=$4
                 RETURNING logical_store_id, application_responsibility, operation_id,
                           step_id, position, step_kind, prerequisites_json, declaration_json,
                           state, outcome_json, recovery_json, created_at_ms, updated_at_ms,
                           completed_at_ms",
                &[
                    &logical_store_id,
                    &application_responsibility,
                    &operation_id,
                    &step_id,
                    &state,
                    &outcome_json,
                    &recovery_json,
                    &now,
                ],
            )
            .await?
            .ok_or_else(|| anyhow!("compound step does not exist"))?;
        pg_tx_append_state_delta(
            &tx,
            "compound",
            &format!("operation:{operation_id}:step:{step_id}"),
            &format!(
                "{{\"operation_id\":{},\"step_id\":{},\"state\":{}}}",
                serde_json::to_string(operation_id)?,
                serde_json::to_string(step_id)?,
                serde_json::to_string(state)?
            ),
        )
        .await?;
        let row = map_compound_step(&row);
        tx.commit().await?;
        Ok(row)
    }

    async fn append_state_delta(
        &self,
        axis: &str,
        entity_id: &str,
        payload_json: &str,
    ) -> Result<StateDeltaRecord> {
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        let record = pg_tx_append_state_delta(&tx, axis, entity_id, payload_json).await?;
        tx.commit().await?;
        Ok(record)
    }

    async fn current_state_version(&self) -> Result<i64> {
        let client = self.client().await?;
        Ok(client
            .query_one(
                "SELECT version FROM application_state_version WHERE singleton=1",
                &[],
            )
            .await?
            .get("version"))
    }

    async fn state_deltas(&self, after_version: i64, limit: i64) -> Result<Vec<StateDeltaRecord>> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT version, axis, entity_id, payload_json, at_ms
                 FROM application_state_deltas
                 WHERE version>$1 ORDER BY version LIMIT $2",
                &[&after_version, &limit.max(1)],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| StateDeltaRecord {
                version: r.get("version"),
                axis: r.get("axis"),
                entity_id: r.get("entity_id"),
                payload_json: r.get("payload_json"),
                at_ms: r.get("at_ms"),
            })
            .collect())
    }

    async fn state_delta_page(
        &self,
        after_version: i64,
        limit: i64,
    ) -> Result<StateDeltaPageRecord> {
        let mut client = self.client().await?;
        let tx = client
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
            .start()
            .await?;
        let current_version: i64 = tx
            .query_one(
                "SELECT version FROM application_state_version WHERE singleton=1",
                &[],
            )
            .await?
            .get("version");
        let retained_floor: Option<i64> = tx
            .query_one(
                "SELECT MIN(version) AS floor FROM application_state_deltas",
                &[],
            )
            .await?
            .get("floor");
        let rows = tx
            .query(
                "SELECT version, axis, entity_id, payload_json, at_ms
                 FROM application_state_deltas
                 WHERE version>$1 ORDER BY version LIMIT $2",
                &[&after_version, &limit.max(1)],
            )
            .await?;
        let deltas = rows
            .iter()
            .map(|row| StateDeltaRecord {
                version: row.get("version"),
                axis: row.get("axis"),
                entity_id: row.get("entity_id"),
                payload_json: row.get("payload_json"),
                at_ms: row.get("at_ms"),
            })
            .collect();
        tx.commit().await?;
        Ok(StateDeltaPageRecord {
            current_version,
            retained_floor: retained_floor.unwrap_or(current_version.saturating_add(1)),
            deltas,
        })
    }

    async fn history_page(&self, query: &HistoryQuery) -> Result<Vec<HistoryRecord>> {
        if query.unresolved_only && query.recipient.is_none() {
            bail!("unresolved history requires an exact recipient");
        }
        let order = match query.order {
            HistoryOrder::Ascending => "ASC",
            HistoryOrder::Descending => "DESC",
        };
        let sql = format!(
            "SELECT {MSG_COLS_M},
                    d.id AS delivery_id,
                    d.message_id AS delivery_message_id,
                    d.recipient AS delivery_recipient,
                    d.occupant AS delivery_occupant,
                    d.delivered_at_ms AS delivery_at_ms,
                    d.consumed_at_ms AS delivery_consumed_at_ms,
                    (SELECT disp.state FROM dispositions disp
                     WHERE disp.message_id=m.id AND disp.recipient=$1
                     ORDER BY disp.id DESC LIMIT 1) AS latest_disp
             FROM messages m
             LEFT JOIN deliveries d
               ON d.message_id=m.id AND $1::text IS NOT NULL AND d.recipient=$1
             WHERE ($1::text IS NULL OR d.id IS NOT NULL)
               AND (NOT $2 OR (
                    m.to_addr=$1 AND m.requires_disposition=TRUE
                    AND COALESCE((SELECT disp.state FROM dispositions disp
                                  WHERE disp.message_id=m.id AND disp.recipient=$1
                                  ORDER BY disp.id DESC LIMIT 1), '') NOT IN ({})
               ))
               AND ($3::bigint IS NULL OR m.thread_id=$3 OR m.id=$3)
               AND ($4::bigint IS NULL OR m.created_at_ms >= $4)
               AND ($5::bigint IS NULL OR m.id > $5)
             ORDER BY m.id {order}
             LIMIT $6",
            terminal_dispositions_sql_list()
        );
        let recipient = query.recipient.as_deref();
        let client = self.client().await?;
        let rows = client
            .query(
                &sql,
                &[
                    &recipient,
                    &query.unresolved_only,
                    &query.thread_id,
                    &query.since_ms,
                    &query.after_message_id,
                    &query.limit.max(1),
                ],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                let delivery_id: Option<i64> = r.get("delivery_id");
                HistoryRecord {
                    message: map_message(r),
                    delivery: delivery_id.map(|id| DeliveryRow {
                        id,
                        message_id: r.get("delivery_message_id"),
                        recipient: r.get("delivery_recipient"),
                        occupant: r.get("delivery_occupant"),
                        delivered_at_ms: r.get("delivery_at_ms"),
                        consumed_at_ms: r.get("delivery_consumed_at_ms"),
                    }),
                    latest_disposition: r.get("latest_disp"),
                }
            })
            .collect())
    }

    async fn cleanup_application_records(
        &self,
        scope: &ApplicationRecordScope,
        policy: RetentionPolicy,
    ) -> Result<CleanupReport> {
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        let deleted_operation_ids: Vec<String> = tx
            .query(
                "SELECT o.operation_id FROM application_operations o
                 WHERE o.logical_store_id=$1 AND o.application_responsibility=$2
                   AND o.state IN ('accepted','rejected','duplicate','completed')
                   AND o.completed_at_ms IS NOT NULL AND o.completed_at_ms<$3
                   AND NOT EXISTS (
                       SELECT 1 FROM application_compound_steps s
                       WHERE s.logical_store_id=o.logical_store_id
                         AND s.application_responsibility=o.application_responsibility
                         AND s.operation_id=o.operation_id
                         AND s.completed_at_ms IS NULL
                   )
                 ORDER BY o.completed_at_ms LIMIT $4
                 FOR UPDATE",
                &[
                    &scope.logical_store_id,
                    &scope.application_responsibility,
                    &policy.completed_before_ms,
                    &policy.max_delete.max(0),
                ],
            )
            .await?
            .iter()
            .map(|row| row.get("operation_id"))
            .collect();
        for operation_id in &deleted_operation_ids {
            tx.execute(
                "DELETE FROM application_operation_messages
                 WHERE logical_store_id=$1 AND application_responsibility=$2
                   AND operation_id=$3",
                &[
                    &scope.logical_store_id,
                    &scope.application_responsibility,
                    operation_id,
                ],
            )
            .await?;
            tx.execute(
                "DELETE FROM application_operations
                 WHERE logical_store_id=$1 AND application_responsibility=$2
                   AND operation_id=$3",
                &[
                    &scope.logical_store_id,
                    &scope.application_responsibility,
                    operation_id,
                ],
            )
            .await?;
        }
        let operations_deleted = deleted_operation_ids.len() as i64;
        let compound_steps_deleted = tx
            .execute(
                "WITH doomed AS (
                     SELECT s.ctid FROM application_compound_steps s
                     WHERE s.logical_store_id=$1 AND s.application_responsibility=$2
                       AND s.completed_at_ms IS NOT NULL AND s.completed_at_ms<$3
                       AND NOT EXISTS (
                           SELECT 1 FROM application_compound_steps pending
                           WHERE pending.logical_store_id=s.logical_store_id
                             AND pending.application_responsibility=s.application_responsibility
                             AND pending.operation_id=s.operation_id
                             AND pending.completed_at_ms IS NULL
                       )
                     ORDER BY completed_at_ms LIMIT $4
                 )
                 DELETE FROM application_compound_steps
                 WHERE ctid IN (SELECT ctid FROM doomed)",
                &[
                    &scope.logical_store_id,
                    &scope.application_responsibility,
                    &policy.completed_before_ms,
                    &policy.max_delete.max(0),
                ],
            )
            .await? as i64;
        tx.commit().await?;
        Ok(CleanupReport {
            operations_deleted,
            compound_steps_deleted,
        })
    }

    async fn application_storage_stats(
        &self,
        scope: &ApplicationRecordScope,
    ) -> Result<ApplicationStorageStats> {
        let client = self.client().await?;
        let operations = client
            .query_one(
                "SELECT COUNT(*)::bigint AS count, MIN(created_at_ms) AS oldest
                 FROM application_operations
                 WHERE logical_store_id=$1 AND application_responsibility=$2",
                &[&scope.logical_store_id, &scope.application_responsibility],
            )
            .await?;
        let compounds = client
            .query_one(
                "SELECT COUNT(*)::bigint AS count, MIN(created_at_ms) AS oldest
                 FROM application_compound_steps
                 WHERE logical_store_id=$1 AND application_responsibility=$2",
                &[&scope.logical_store_id, &scope.application_responsibility],
            )
            .await?;
        Ok(ApplicationStorageStats {
            operation_rows: operations.get("count"),
            compound_step_rows: compounds.get("count"),
            oldest_operation_at_ms: operations.get("oldest"),
            oldest_compound_step_at_ms: compounds.get("oldest"),
        })
    }

    async fn cleanup_state_deltas(
        &self,
        policy: StoreDeltaRetentionPolicy,
    ) -> Result<StoreDeltaCleanupReport> {
        let client = self.client().await?;
        let deleted = client
            .execute(
                "WITH doomed AS (
                     SELECT version FROM application_state_deltas
                     WHERE version<$1 ORDER BY version LIMIT $2
                 )
                 DELETE FROM application_state_deltas
                 WHERE version IN (SELECT version FROM doomed)",
                &[&policy.before_version, &policy.max_delete.max(0)],
            )
            .await? as i64;
        Ok(StoreDeltaCleanupReport {
            deltas_deleted: deleted,
        })
    }

    async fn undelivered_counts(&self) -> Result<Vec<(String, i64)>> {
        let client = self.client().await?;
        let sql = format!(
            "SELECT m.to_addr, COUNT(*) AS n FROM messages m \
             WHERE NOT EXISTS (SELECT 1 FROM deliveries d \
                               WHERE d.message_id=m.id AND d.recipient=m.to_addr \
                                 AND d.consumed_at_ms IS NOT NULL) \
               AND COALESCE((SELECT disp.state FROM dispositions disp \
                             WHERE disp.message_id=m.id AND disp.recipient=m.to_addr \
                             ORDER BY disp.id DESC LIMIT 1), '') NOT IN ({}) \
             GROUP BY m.to_addr",
            terminal_dispositions_sql_list()
        );
        let rows = client.query(sql.as_str(), &[]).await?;
        Ok(rows
            .iter()
            .map(|r| (r.get::<_, String>("to_addr"), r.get::<_, i64>("n")))
            .collect())
    }

    async fn feed_page(&self, after_id: i64, limit: i64) -> Result<Vec<MessageRow>> {
        let client = self.client().await?;
        let sql = format!("SELECT {MSG_COLS} FROM messages WHERE id>$1 ORDER BY id LIMIT $2");
        let rows = client.query(sql.as_str(), &[&after_id, &limit]).await?;
        Ok(rows.iter().map(map_message).collect())
    }

    async fn notify_new(&self, address: &str, id: i64, sent_at_ms: i64) -> Result<()> {
        let payload =
            serde_json::json!({"address": address, "id": id, "sent_at_ms": sent_at_ms}).to_string();
        let client = self.client().await?;
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
