//! SQLite backend: the zero-config local substrate. WAL mode plus a busy timeout
//! make multiple processes (holders, senders, waiters) safe on one file. Liveness is
//! TTL-heartbeat; delivery is poll-with-cursor. Validated for multi-process use in the spike.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::{Arc, Mutex};

use super::{Backend, Capabilities};
use crate::model::*;

pub struct SqliteBackend {
    conn: Arc<Mutex<Connection>>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS addresses (
    address       TEXT PRIMARY KEY,
    description   TEXT,
    scope         TEXT,
    tags          TEXT,
    status        TEXT NOT NULL DEFAULT 'active',
    created_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS leases (
    address         TEXT PRIMARY KEY,
    occupant        TEXT,
    host            TEXT,
    principal       TEXT,
    description     TEXT,
    tags            TEXT,
    scope           TEXT,
    pid             INTEGER,
    since_ms        INTEGER NOT NULL,
    heartbeat_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS messages (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id     INTEGER,
    parent_id     INTEGER,
    from_addr     TEXT,
    to_addr       TEXT NOT NULL,
    cc            TEXT,
    kind          TEXT NOT NULL DEFAULT 'note',
    attention     TEXT NOT NULL DEFAULT 'background',
    requires_disposition INTEGER NOT NULL DEFAULT 0,
    subject       TEXT,
    body          TEXT NOT NULL,
    metadata      TEXT,
    sent_at_ms    INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_to_id_idx ON messages(to_addr, id);
CREATE INDEX IF NOT EXISTS messages_thread_idx ON messages(thread_id, id);
CREATE TABLE IF NOT EXISTS dispositions (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id   INTEGER NOT NULL,
    recipient    TEXT NOT NULL,
    state        TEXT NOT NULL,
    note         TEXT,
    by_principal TEXT,
    at_ms        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS dispositions_msg_idx ON dispositions(message_id, id);
CREATE TABLE IF NOT EXISTS deliveries (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id      INTEGER NOT NULL,
    recipient       TEXT NOT NULL,
    occupant        TEXT,
    delivered_at_ms INTEGER NOT NULL,
    UNIQUE(message_id, recipient)
);
"#;

/// Column list used by every message SELECT so row mapping stays positional and stable.
const MSG_COLS: &str = "id, thread_id, parent_id, from_addr, to_addr, cc, kind, attention, \
    requires_disposition, subject, body, metadata, sent_at_ms, created_at_ms";

fn map_message(r: &rusqlite::Row) -> rusqlite::Result<MessageRow> {
    let id: i64 = r.get(0)?;
    let thread_id: Option<i64> = r.get(1)?;
    Ok(MessageRow {
        id,
        thread_id: thread_id.unwrap_or(id),
        parent_id: r.get(2)?,
        from_addr: r.get(3)?,
        to_addr: r.get(4)?,
        cc: r.get(5)?,
        kind: r.get(6)?,
        attention: r.get(7)?,
        requires_disposition: r.get::<_, i64>(8)? != 0,
        subject: r.get(9)?,
        body: r.get(10)?,
        metadata: r.get(11)?,
        sent_at_ms: r.get(12)?,
        created_at_ms: r.get(13)?,
    })
}

impl SqliteBackend {
    pub fn open(path: &str) -> Result<Self> {
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).ok();
            }
        }
        let conn = Connection::open(path)?;
        // Set busy_timeout *before* the journal_mode=WAL switch: that switch briefly takes a
        // write lock, so when several connections open the same fresh database at once
        // (multiple holders/senders starting together) a still-default zero timeout makes the
        // contended opener fail with a spurious "database is locked" instead of waiting. This
        // greatly reduces such startup errors — though it is not an absolute guarantee, since
        // SQLite skips the busy handler on a simultaneous SHARED->EXCLUSIVE WAL promotion to
        // avoid deadlock. The backend conformance concurrency scenario exercises this path.
        conn.execute_batch(
            "PRAGMA busy_timeout=5000; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;",
        )?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    async fn run<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().unwrap();
            f(&guard)
        })
        .await?
    }
}

#[async_trait]
impl Backend for SqliteBackend {
    fn kind(&self) -> &'static str {
        "sqlite"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            durable: true,
            push: "poll",
            lease: "ttl",
        }
    }

    async fn init_schema(&self) -> Result<()> {
        self.run(|c| {
            c.execute_batch(SCHEMA)?;
            Ok(())
        })
        .await
    }

    async fn ensure_address(
        &self,
        address: &str,
        description: Option<&str>,
        scope: Option<&str>,
        tags: Option<&str>,
    ) -> Result<()> {
        let (a, d, s, t) = (
            address.to_string(),
            description.map(str::to_string),
            scope.map(str::to_string),
            tags.map(str::to_string),
        );
        let now = now_ms();
        self.run(move |c| {
            // Insert if absent; otherwise refresh non-null descriptive fields.
            c.execute(
                "INSERT INTO addresses(address, description, scope, tags, status, created_at_ms) \
                 VALUES (?1,?2,?3,?4,'active',?5) \
                 ON CONFLICT(address) DO UPDATE SET \
                    description=COALESCE(excluded.description, addresses.description), \
                    scope=COALESCE(excluded.scope, addresses.scope), \
                    tags=COALESCE(excluded.tags, addresses.tags)",
                params![a, d, s, t, now],
            )?;
            Ok(())
        })
        .await
    }

    async fn get_address(&self, address: &str) -> Result<Option<AddressRow>> {
        let a = address.to_string();
        self.run(move |c| {
            let row = c
                .query_row(
                    "SELECT address, description, scope, tags, status, created_at_ms \
                     FROM addresses WHERE address=?1",
                    params![a],
                    |r| {
                        Ok(AddressRow {
                            address: r.get(0)?,
                            description: r.get(1)?,
                            scope: r.get(2)?,
                            tags: r.get(3)?,
                            status: r.get(4)?,
                            created_at_ms: r.get(5)?,
                        })
                    },
                )
                .optional()?;
            Ok(row)
        })
        .await
    }

    async fn set_address_status(&self, address: &str, status: &str) -> Result<bool> {
        let (a, s) = (address.to_string(), status.to_string());
        self.run(move |c| {
            let n = c.execute(
                "UPDATE addresses SET status=?2 WHERE address=?1",
                params![a, s],
            )?;
            Ok(n > 0)
        })
        .await
    }

    async fn list_addresses(
        &self,
        scope: Option<&str>,
        include_retired: bool,
    ) -> Result<Vec<AddressRow>> {
        let scope = scope.map(str::to_string);
        self.run(move |c| {
            let mut sql = String::from(
                "SELECT address, description, scope, tags, status, created_at_ms FROM addresses WHERE 1=1",
            );
            if !include_retired {
                sql.push_str(" AND status='active'");
            }
            if scope.is_some() {
                sql.push_str(" AND scope=?1");
            }
            sql.push_str(" ORDER BY address");
            let mut stmt = c.prepare(&sql)?;
            let map = |r: &rusqlite::Row| {
                Ok(AddressRow {
                    address: r.get(0)?,
                    description: r.get(1)?,
                    scope: r.get(2)?,
                    tags: r.get(3)?,
                    status: r.get(4)?,
                    created_at_ms: r.get(5)?,
                })
            };
            let rows: Vec<AddressRow> = if let Some(s) = scope {
                stmt.query_map(params![s], map)?
                    .collect::<rusqlite::Result<_>>()?
            } else {
                stmt.query_map([], map)?.collect::<rusqlite::Result<_>>()?
            };
            Ok(rows)
        })
        .await
    }

    async fn claim_lease(&self, claim: &LeaseClaim, window_secs: i64) -> Result<LeaseOutcome> {
        let claim = claim.clone();
        self.run(move |c| {
            c.execute_batch("BEGIN IMMEDIATE;")?;
            let result = (|| -> Result<LeaseOutcome> {
                let now = now_ms();
                let existing = c
                    .query_row(
                        "SELECT occupant, since_ms, heartbeat_at_ms FROM leases WHERE address=?1",
                        params![claim.address],
                        |r| {
                            Ok((
                                r.get::<_, Option<String>>(0)?,
                                r.get::<_, i64>(1)?,
                                r.get::<_, i64>(2)?,
                            ))
                        },
                    )
                    .optional()?;

                if let Some((occ, _since, hb)) = &existing {
                    let live = now - *hb < window_secs * 1000;
                    let same = occ.as_deref() == Some(claim.occupant.as_str());
                    if live && !same {
                        let lease = read_lease(c, &claim.address)?;
                        return Ok(LeaseOutcome::AlreadyOccupied(
                            lease.unwrap_or_else(|| placeholder_lease(&claim.address, occ.clone())),
                        ));
                    }
                }
                let since = match &existing {
                    Some((occ, since, _)) if occ.as_deref() == Some(claim.occupant.as_str()) => {
                        *since
                    }
                    _ => now,
                };
                c.execute(
                    "INSERT INTO leases(address, occupant, host, principal, description, tags, scope, pid, since_ms, heartbeat_at_ms) \
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) \
                     ON CONFLICT(address) DO UPDATE SET occupant=excluded.occupant, host=excluded.host, \
                        principal=excluded.principal, description=excluded.description, tags=excluded.tags, \
                        scope=excluded.scope, pid=excluded.pid, since_ms=excluded.since_ms, \
                        heartbeat_at_ms=excluded.heartbeat_at_ms",
                    params![
                        claim.address, claim.occupant, claim.host, claim.principal,
                        claim.description, claim.tags, claim.scope, claim.pid, since, now
                    ],
                )?;
                Ok(LeaseOutcome::Claimed)
            })();
            match &result {
                Ok(_) => c.execute_batch("COMMIT;")?,
                Err(_) => c.execute_batch("ROLLBACK;")?,
            }
            result
        })
        .await
    }

    async fn heartbeat(&self, address: &str) -> Result<()> {
        let a = address.to_string();
        let now = now_ms();
        self.run(move |c| {
            c.execute(
                "UPDATE leases SET heartbeat_at_ms=?2 WHERE address=?1",
                params![a, now],
            )?;
            Ok(())
        })
        .await
    }

    async fn release_lease(&self, address: &str, occupant: &str) -> Result<bool> {
        let (a, o) = (address.to_string(), occupant.to_string());
        self.run(move |c| {
            let n = c.execute(
                "DELETE FROM leases WHERE address=?1 AND (occupant=?2 OR occupant IS NULL)",
                params![a, o],
            )?;
            Ok(n > 0)
        })
        .await
    }

    async fn get_lease(&self, address: &str) -> Result<Option<LeaseRow>> {
        let a = address.to_string();
        self.run(move |c| read_lease(c, &a)).await
    }

    async fn occupancy(&self, address: &str, window_secs: i64) -> Result<Occupancy> {
        let a = address.to_string();
        let now = now_ms();
        self.run(move |c| {
            let row = c
                .query_row(
                    "SELECT occupant, heartbeat_at_ms FROM leases WHERE address=?1",
                    params![a],
                    |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?)),
                )
                .optional()?;
            Ok(match row {
                None => Occupancy {
                    occupied: false,
                    age_secs: 0.0,
                    occupant: None,
                },
                Some((occupant, hb)) => {
                    let age_ms = now - hb;
                    Occupancy {
                        occupied: age_ms < window_secs * 1000,
                        age_secs: age_ms as f64 / 1000.0,
                        occupant,
                    }
                }
            })
        })
        .await
    }

    async fn max_id(&self, address: &str) -> Result<i64> {
        let a = address.to_string();
        self.run(move |c| {
            Ok(c.query_row(
                "SELECT COALESCE(MAX(id),0) FROM messages WHERE to_addr=?1",
                params![a],
                |r| r.get(0),
            )?)
        })
        .await
    }

    async fn max_message_id(&self) -> Result<i64> {
        self.run(move |c| {
            Ok(c.query_row("SELECT COALESCE(MAX(id),0) FROM messages", [], |r| r.get(0))?)
        })
        .await
    }

    async fn fetch_after(&self, address: &str, cursor: i64) -> Result<Vec<MessageRow>> {
        let a = address.to_string();
        self.run(move |c| {
            let sql =
                format!("SELECT {MSG_COLS} FROM messages WHERE to_addr=?1 AND id>?2 ORDER BY id");
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt
                .query_map(params![a, cursor], map_message)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
    }

    async fn mark_delivered(
        &self,
        message_id: i64,
        recipient: &str,
        occupant: Option<&str>,
    ) -> Result<()> {
        let (r, o) = (recipient.to_string(), occupant.map(str::to_string));
        let now = now_ms();
        self.run(move |c| {
            c.execute(
                "INSERT INTO deliveries(message_id, recipient, occupant, delivered_at_ms) \
                 VALUES (?1,?2,?3,?4) ON CONFLICT(message_id, recipient) DO NOTHING",
                params![message_id, r, o, now],
            )?;
            Ok(())
        })
        .await
    }

    async fn undelivered_backlog(&self, address: &str, upto_id: i64) -> Result<Vec<MessageRow>> {
        let a = address.to_string();
        self.run(move |c| {
            // Backlog = messages addressed here, at or below the holder's start cursor, that have no
            // delivery record AND whose latest disposition for this recipient is not terminal. The
            // `id <= upto_id` bound partitions cleanly against the `fetch_after` (id > cursor) drain,
            // so a message inserted between the cursor snapshot and this query is drained, not seeded.
            let sql = format!(
                "SELECT {MSG_COLS} FROM messages m \
                 WHERE m.to_addr=?1 AND m.id<=?2 \
                   AND NOT EXISTS (SELECT 1 FROM deliveries d \
                                   WHERE d.message_id=m.id AND d.recipient=?1) \
                   AND COALESCE((SELECT disp.state FROM dispositions disp \
                                 WHERE disp.message_id=m.id AND disp.recipient=?1 \
                                 ORDER BY disp.id DESC LIMIT 1), '') NOT IN ({}) \
                 ORDER BY m.id",
                terminal_dispositions_sql_list()
            );
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt
                .query_map(params![a, upto_id], map_message)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
    }

    async fn insert_message(&self, m: &NewMessage) -> Result<MessageRow> {
        let m = m.clone();
        self.run(move |c| {
            c.execute_batch("BEGIN IMMEDIATE;")?;
            let result = (|| -> Result<MessageRow> {
                let now = now_ms();
                c.execute(
                    "INSERT INTO messages(thread_id, parent_id, from_addr, to_addr, cc, kind, attention, \
                        requires_disposition, subject, body, metadata, sent_at_ms, created_at_ms) \
                     VALUES (NULL,?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                    params![
                        m.parent_id, m.from_addr, m.to_addr, m.cc, m.kind, m.attention.as_str(),
                        m.requires_disposition as i64, m.subject, m.body, m.metadata, m.sent_at_ms, now
                    ],
                )?;
                let id = c.last_insert_rowid();
                // Resolve thread: inherit the parent's thread, else this message roots its own.
                let thread_id: i64 = match m.parent_id {
                    Some(pid) => c
                        .query_row(
                            "SELECT COALESCE(thread_id, id) FROM messages WHERE id=?1",
                            params![pid],
                            |r| r.get(0),
                        )
                        .optional()?
                        .unwrap_or(id),
                    None => id,
                };
                c.execute(
                    "UPDATE messages SET thread_id=?2 WHERE id=?1",
                    params![id, thread_id],
                )?;
                let row = c.query_row(
                    &format!("SELECT {MSG_COLS} FROM messages WHERE id=?1"),
                    params![id],
                    map_message,
                )?;
                Ok(row)
            })();
            match &result {
                Ok(_) => c.execute_batch("COMMIT;")?,
                Err(_) => c.execute_batch("ROLLBACK;")?,
            }
            result
        })
        .await
    }

    async fn get_message(&self, id: i64) -> Result<Option<MessageRow>> {
        self.run(move |c| {
            let row = c
                .query_row(
                    &format!("SELECT {MSG_COLS} FROM messages WHERE id=?1"),
                    params![id],
                    map_message,
                )
                .optional()?;
            Ok(row)
        })
        .await
    }

    async fn thread_messages(&self, thread_id: i64) -> Result<Vec<MessageRow>> {
        self.run(move |c| {
            let sql =
                format!("SELECT {MSG_COLS} FROM messages WHERE thread_id=?1 OR id=?1 ORDER BY id");
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt
                .query_map(params![thread_id], map_message)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
    }

    async fn inbox(&self, address: &str, include_all: bool, limit: i64) -> Result<Vec<InboxItem>> {
        let a = address.to_string();
        self.run(move |c| {
            let sql = format!(
                "SELECT {MSG_COLS}, \
                    (SELECT d.state FROM dispositions d WHERE d.message_id=messages.id \
                       AND d.recipient=?1 ORDER BY d.id DESC LIMIT 1) AS latest_disp \
                 FROM messages WHERE to_addr=?1 ORDER BY id DESC LIMIT ?2"
            );
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt
                .query_map(params![a, limit], |r| {
                    let msg = map_message(r)?;
                    let latest: Option<String> = r.get(14)?;
                    Ok((msg, latest))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let items: Vec<InboxItem> = rows
                .into_iter()
                .map(|(message, latest)| {
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
        })
        .await
    }

    async fn export(
        &self,
        address: Option<&str>,
        thread: Option<i64>,
        since: i64,
    ) -> Result<Vec<MessageRow>> {
        let a = address.map(str::to_string);
        self.run(move |c| {
            let mut sql = format!("SELECT {MSG_COLS} FROM messages WHERE id>?1");
            if a.is_some() {
                sql.push_str(" AND (to_addr=?2 OR from_addr=?2)");
            }
            if let Some(t) = thread {
                sql.push_str(&format!(" AND (thread_id={t} OR id={t})"));
            }
            sql.push_str(" ORDER BY id");
            let mut stmt = c.prepare(&sql)?;
            let rows = if let Some(addr) = a {
                stmt.query_map(params![since, addr], map_message)?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            } else {
                stmt.query_map(params![since], map_message)?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            Ok(rows)
        })
        .await
    }

    async fn insert_disposition(
        &self,
        message_id: i64,
        recipient: &str,
        state: &str,
        note: Option<&str>,
        by: Option<&str>,
    ) -> Result<DispositionRow> {
        let (r, s, n, b) = (
            recipient.to_string(),
            state.to_string(),
            note.map(str::to_string),
            by.map(str::to_string),
        );
        self.run(move |c| {
            let now = now_ms();
            c.execute(
                "INSERT INTO dispositions(message_id, recipient, state, note, by_principal, at_ms) \
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![message_id, r, s, n, b, now],
            )?;
            let id = c.last_insert_rowid();
            Ok(DispositionRow {
                id,
                message_id,
                recipient: r,
                state: s,
                note: n,
                by_principal: b,
                at_ms: now,
            })
        })
        .await
    }

    async fn dispositions_for(&self, message_id: i64) -> Result<Vec<DispositionRow>> {
        self.run(move |c| {
            let mut stmt = c.prepare(
                "SELECT id, message_id, recipient, state, note, by_principal, at_ms \
                 FROM dispositions WHERE message_id=?1 ORDER BY id",
            )?;
            let rows = stmt
                .query_map(params![message_id], |r| {
                    Ok(DispositionRow {
                        id: r.get(0)?,
                        message_id: r.get(1)?,
                        recipient: r.get(2)?,
                        state: r.get(3)?,
                        note: r.get(4)?,
                        by_principal: r.get(5)?,
                        at_ms: r.get(6)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
    }

    async fn notify_new(&self, _address: &str, _id: i64, _sent_at_ms: i64) -> Result<()> {
        Ok(()) // no native push; poll covers it
    }
}

fn read_lease(c: &Connection, address: &str) -> Result<Option<LeaseRow>> {
    let row = c
        .query_row(
            "SELECT address, occupant, host, principal, description, tags, scope, pid, since_ms, heartbeat_at_ms \
             FROM leases WHERE address=?1",
            params![address],
            |r| {
                Ok(LeaseRow {
                    address: r.get(0)?,
                    occupant: r.get(1)?,
                    host: r.get(2)?,
                    principal: r.get(3)?,
                    description: r.get(4)?,
                    tags: r.get(5)?,
                    scope: r.get(6)?,
                    pid: r.get(7)?,
                    since_ms: r.get(8)?,
                    heartbeat_at_ms: r.get(9)?,
                })
            },
        )
        .optional()
        .map_err(|e| anyhow!(e))?;
    Ok(row)
}

fn placeholder_lease(address: &str, occupant: Option<String>) -> LeaseRow {
    LeaseRow {
        address: address.to_string(),
        occupant,
        host: None,
        principal: None,
        description: None,
        tags: None,
        scope: None,
        pid: None,
        since_ms: 0,
        heartbeat_at_ms: 0,
    }
}
