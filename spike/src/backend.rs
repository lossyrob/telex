//! Backend abstraction for the spike: one `Backend` trait, two implementations
//! (Postgres and SQLite), so the same holder/waiter/sender code runs over either.
//! This is the "same semantic core, two backends" promise (decision 0005) made
//! concrete enough to prove SQLite multi-process concurrency for the local case.

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

use crate::{connect, now_ms, NOTIFY_CHANNEL, SCHEMA};

#[derive(Clone, Debug)]
pub struct MsgRow {
    pub id: i64,
    pub address: String,
    pub body: String,
    pub attention: String,
    pub sent_at_ms: i64,
}

#[derive(Debug)]
pub struct Occupancy {
    pub occupied: bool,
    pub age_secs: f64,
    pub occupant: Option<String>,
}

#[async_trait]
pub trait Backend: Send + Sync {
    fn kind(&self) -> &'static str;
    async fn init_schema(&self) -> Result<()>;
    async fn ensure_address(&self, address: &str, description: &str) -> Result<()>;
    async fn claim_lease(
        &self,
        address: &str,
        occupant: &str,
        host: &str,
        principal: &str,
    ) -> Result<()>;
    async fn heartbeat(&self, address: &str) -> Result<()>;
    async fn max_id(&self, address: &str) -> Result<i64>;
    async fn fetch_after(&self, address: &str, cursor: i64) -> Result<Vec<MsgRow>>;
    async fn insert_message(
        &self,
        address: &str,
        body: &str,
        attention: &str,
        sent_at_ms: i64,
    ) -> Result<i64>;
    /// Best-effort push signal. No-op where the backend has no native push.
    async fn notify_new(&self, address: &str, id: i64, sent_at_ms: i64) -> Result<()>;
    async fn occupancy(&self, address: &str, window_secs: i64) -> Result<Occupancy>;
}

pub async fn make_backend(kind: &str, db_path: &str) -> Result<Arc<dyn Backend>> {
    match kind {
        "postgres" | "pg" => {
            let b = PgBackend::connect().await?;
            b.init_schema().await?;
            Ok(Arc::new(b))
        }
        "sqlite" => {
            let b = SqliteBackend::open(db_path)?;
            b.init_schema().await?;
            Ok(Arc::new(b))
        }
        other => bail!("unknown backend: {other}"),
    }
}

// ---------------- Postgres ----------------

pub struct PgBackend {
    client: tokio_postgres::Client,
    notify_client: tokio_postgres::Client,
}

impl PgBackend {
    pub async fn connect() -> Result<Self> {
        Ok(Self {
            client: connect().await?,
            notify_client: connect().await?,
        })
    }
}

#[async_trait]
impl Backend for PgBackend {
    fn kind(&self) -> &'static str {
        "postgres"
    }
    async fn init_schema(&self) -> Result<()> {
        self.client.batch_execute(SCHEMA).await?;
        Ok(())
    }
    async fn ensure_address(&self, address: &str, description: &str) -> Result<()> {
        self.client
            .execute(
                "INSERT INTO addresses(address, description) VALUES ($1,$2) \
                 ON CONFLICT (address) DO NOTHING",
                &[&address, &description],
            )
            .await?;
        Ok(())
    }
    async fn claim_lease(
        &self,
        address: &str,
        occupant: &str,
        host: &str,
        principal: &str,
    ) -> Result<()> {
        self.client
            .execute(
                "INSERT INTO leases(address, occupant, host, principal, heartbeat_at) \
                 VALUES ($1,$2,$3,$4, now()) \
                 ON CONFLICT (address) DO UPDATE SET occupant=excluded.occupant, \
                     host=excluded.host, principal=excluded.principal, heartbeat_at=now()",
                &[&address, &occupant, &host, &principal],
            )
            .await?;
        Ok(())
    }
    async fn heartbeat(&self, address: &str) -> Result<()> {
        self.client
            .execute("UPDATE leases SET heartbeat_at = now() WHERE address=$1", &[&address])
            .await?;
        Ok(())
    }
    async fn max_id(&self, address: &str) -> Result<i64> {
        Ok(self
            .client
            .query_one(
                "SELECT COALESCE(MAX(id),0) m FROM messages WHERE address=$1",
                &[&address],
            )
            .await?
            .get("m"))
    }
    async fn fetch_after(&self, address: &str, cursor: i64) -> Result<Vec<MsgRow>> {
        let rows = self
            .client
            .query(
                "SELECT id, address, body, attention, COALESCE(sent_at_ms,0) AS sent_at_ms \
                 FROM messages WHERE address=$1 AND id>$2 ORDER BY id",
                &[&address, &cursor],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| MsgRow {
                id: r.get("id"),
                address: r.get("address"),
                body: r.get("body"),
                attention: r.get("attention"),
                sent_at_ms: r.get("sent_at_ms"),
            })
            .collect())
    }
    async fn insert_message(
        &self,
        address: &str,
        body: &str,
        attention: &str,
        sent_at_ms: i64,
    ) -> Result<i64> {
        Ok(self
            .client
            .query_one(
                "INSERT INTO messages(address, body, attention, sent_at_ms) \
                 VALUES ($1,$2,$3,$4) RETURNING id",
                &[&address, &body, &attention, &sent_at_ms],
            )
            .await?
            .get("id"))
    }
    async fn notify_new(&self, address: &str, id: i64, sent_at_ms: i64) -> Result<()> {
        let payload =
            serde_json::json!({"address": address, "id": id, "sent_at_ms": sent_at_ms}).to_string();
        self.notify_client
            .execute("SELECT pg_notify($1,$2)", &[&NOTIFY_CHANNEL, &payload])
            .await?;
        Ok(())
    }
    async fn occupancy(&self, address: &str, window_secs: i64) -> Result<Occupancy> {
        let row = self
            .client
            .query_opt(
                "SELECT occupant, \
                        EXTRACT(EPOCH FROM (now()-heartbeat_at))::float8 AS age, \
                        (heartbeat_at > now() - make_interval(secs => $2::double precision)) AS occupied \
                 FROM leases WHERE address=$1",
                &[&address, &(window_secs as f64)],
            )
            .await?;
        Ok(match row {
            None => Occupancy { occupied: false, age_secs: 0.0, occupant: None },
            Some(r) => Occupancy {
                occupied: r.get("occupied"),
                age_secs: r.get("age"),
                occupant: r.get("occupant"),
            },
        })
    }
}

// ---------------- SQLite ----------------

pub struct SqliteBackend {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteBackend {
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        // WAL + busy_timeout are the crux of multi-process concurrency on one file.
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;",
        )?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    async fn run<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().unwrap();
            f(&guard)
        })
        .await?
        .map_err(|e| anyhow!(e))
    }
}

const SQLITE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS addresses (
    address     TEXT PRIMARY KEY,
    description TEXT,
    status      TEXT NOT NULL DEFAULT 'active',
    created_at  INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER))
);
CREATE TABLE IF NOT EXISTS leases (
    address         TEXT PRIMARY KEY,
    occupant        TEXT,
    host            TEXT,
    principal       TEXT,
    heartbeat_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS messages (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    address    TEXT NOT NULL,
    body       TEXT NOT NULL,
    attention  TEXT NOT NULL DEFAULT 'background',
    sent_at_ms INTEGER,
    created_at INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER))
);
CREATE INDEX IF NOT EXISTS messages_address_id_idx ON messages (address, id);
"#;

#[async_trait]
impl Backend for SqliteBackend {
    fn kind(&self) -> &'static str {
        "sqlite"
    }
    async fn init_schema(&self) -> Result<()> {
        self.run(|c| c.execute_batch(SQLITE_SCHEMA)).await
    }
    async fn ensure_address(&self, address: &str, description: &str) -> Result<()> {
        let (a, d) = (address.to_string(), description.to_string());
        self.run(move |c| {
            c.execute(
                "INSERT OR IGNORE INTO addresses(address, description) VALUES (?1, ?2)",
                rusqlite::params![a, d],
            )
            .map(|_| ())
        })
        .await
    }
    async fn claim_lease(
        &self,
        address: &str,
        occupant: &str,
        host: &str,
        principal: &str,
    ) -> Result<()> {
        let (a, o, h, p) = (
            address.to_string(),
            occupant.to_string(),
            host.to_string(),
            principal.to_string(),
        );
        let hb = now_ms();
        self.run(move |c| {
            c.execute(
                "INSERT INTO leases(address, occupant, host, principal, heartbeat_at_ms) \
                 VALUES (?1,?2,?3,?4,?5) \
                 ON CONFLICT(address) DO UPDATE SET occupant=excluded.occupant, \
                     host=excluded.host, principal=excluded.principal, \
                     heartbeat_at_ms=excluded.heartbeat_at_ms",
                rusqlite::params![a, o, h, p, hb],
            )
            .map(|_| ())
        })
        .await
    }
    async fn heartbeat(&self, address: &str) -> Result<()> {
        let a = address.to_string();
        let hb = now_ms();
        self.run(move |c| {
            c.execute(
                "UPDATE leases SET heartbeat_at_ms=?2 WHERE address=?1",
                rusqlite::params![a, hb],
            )
            .map(|_| ())
        })
        .await
    }
    async fn max_id(&self, address: &str) -> Result<i64> {
        let a = address.to_string();
        self.run(move |c| {
            c.query_row(
                "SELECT COALESCE(MAX(id),0) FROM messages WHERE address=?1",
                rusqlite::params![a],
                |r| r.get(0),
            )
        })
        .await
    }
    async fn fetch_after(&self, address: &str, cursor: i64) -> Result<Vec<MsgRow>> {
        let a = address.to_string();
        self.run(move |c| {
            let mut stmt = c.prepare(
                "SELECT id, address, body, attention, COALESCE(sent_at_ms,0) \
                 FROM messages WHERE address=?1 AND id>?2 ORDER BY id",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![a, cursor], |r| {
                    Ok(MsgRow {
                        id: r.get(0)?,
                        address: r.get(1)?,
                        body: r.get(2)?,
                        attention: r.get(3)?,
                        sent_at_ms: r.get(4)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
    }
    async fn insert_message(
        &self,
        address: &str,
        body: &str,
        attention: &str,
        sent_at_ms: i64,
    ) -> Result<i64> {
        let (a, b, att) = (address.to_string(), body.to_string(), attention.to_string());
        self.run(move |c| {
            c.execute(
                "INSERT INTO messages(address, body, attention, sent_at_ms) VALUES (?1,?2,?3,?4)",
                rusqlite::params![a, b, att, sent_at_ms],
            )?;
            Ok(c.last_insert_rowid())
        })
        .await
    }
    async fn notify_new(&self, _address: &str, _id: i64, _sent_at_ms: i64) -> Result<()> {
        Ok(()) // no native push; poll covers it
    }
    async fn occupancy(&self, address: &str, window_secs: i64) -> Result<Occupancy> {
        let a = address.to_string();
        let now = now_ms();
        self.run(move |c| {
            let row = c
                .query_row(
                    "SELECT occupant, heartbeat_at_ms FROM leases WHERE address=?1",
                    rusqlite::params![a],
                    |r| {
                        let occupant: Option<String> = r.get(0)?;
                        let hb: i64 = r.get(1)?;
                        Ok((occupant, hb))
                    },
                )
                .ok();
            Ok(match row {
                None => Occupancy { occupied: false, age_secs: 0.0, occupant: None },
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
}
