//! Backend conformance suite: one shared battery of scenarios that runs against *any*
//! `Backend` implementation. A contributor adding a new backend implements the `Backend`
//! trait, wires up a fixture (a `Store` factory), and runs `cargo test` to prove their
//! backend honours the trait's contract.
//!
//! The scenarios assert only on *observable trait behavior* — never on storage internals —
//! so the same `run_all` battery validates SQLite, Postgres, and any future backend
//! identically. The battery is written generically over a `Store` factory so it can be
//! lifted into a standalone `telex-backend-tests` crate once the modular-backend crate
//! split (DECISIONS 0008) happens.
//!
//! Fixtures:
//! - **SQLite** runs by default in CI: each `Store` is a fresh temp-file database.
//! - **Postgres** runs only when `TELEX_PG_URL` is set, against an isolated schema
//!   (`TELEX_PG_SCHEMA`, default `telex_conformance`, suffixed per run); it is dropped and
//!   recreated per run and *skips cleanly* (does not fail) when no database is configured,
//!   so CI without a Postgres server stays green.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use telex::backend::Backend;
use telex::model::{now_ms, Attention, LeaseClaim, LeaseOutcome, NewMessage};

type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// A handle to one underlying store. `connect` opens a *fresh connection* to the **same**
/// store every time, so scenarios that need multiple concurrent occupants (lease
/// exclusivity, concurrent inserts) can model independent processes. A new `Store`
/// produced by the harness factory always starts empty.
#[derive(Clone)]
struct Store {
    connect: Arc<dyn Fn() -> BoxFut<Arc<dyn Backend>> + Send + Sync>,
}

impl Store {
    async fn connect(&self) -> Arc<dyn Backend> {
        (self.connect)().await
    }
}

// ----------------------------------------------------------------------------------------
// Builders
// ----------------------------------------------------------------------------------------

fn new_msg(to: &str) -> NewMessage {
    NewMessage {
        parent_id: None,
        from_addr: None,
        to_addr: to.to_string(),
        cc: None,
        kind: "note".to_string(),
        attention: Attention::Background,
        requires_disposition: false,
        subject: None,
        body: "body".to_string(),
        metadata: None,
        sent_at_ms: now_ms(),
    }
}

fn new_claim(addr: &str, occupant: &str) -> LeaseClaim {
    LeaseClaim {
        address: addr.to_string(),
        occupant: occupant.to_string(),
        host: "host".to_string(),
        principal: "principal".to_string(),
        description: Some("holder".to_string()),
        tags: None,
        scope: None,
        pid: 1,
    }
}

// ----------------------------------------------------------------------------------------
// The shared battery
// ----------------------------------------------------------------------------------------

/// Run every scenario against fresh, empty stores produced by `make_store`.
async fn run_all<F>(make_store: F)
where
    F: Fn() -> BoxFut<Store>,
{
    schema_idempotent(make_store().await).await;
    addresses_directory(make_store().await).await;
    leases_liveness(make_store().await).await;
    cursor_delivery(make_store().await).await;
    messages_threading(make_store().await).await;
    inbox_derivation(make_store().await).await;
    dispositions(make_store().await).await;
    export_filters(make_store().await).await;
    concurrency(make_store().await).await;
}

/// `init_schema` must be safe to call repeatedly.
async fn schema_idempotent(store: Store) {
    let b = store.connect().await;
    for _ in 0..3 {
        b.init_schema()
            .await
            .expect("init_schema must be idempotent");
    }
    // The store is still usable after repeated init.
    b.ensure_address("idem:1", Some("d"), None, None)
        .await
        .unwrap();
    assert!(b.get_address("idem:1").await.unwrap().is_some());
}

/// Addresses/directory: ensure/get/list; status transitions; retired drops from default
/// listings; scope filtering.
async fn addresses_directory(store: Store) {
    let b = store.connect().await;

    b.ensure_address("addr:1", Some("desc"), Some("scope:x"), Some("t1,t2"))
        .await
        .unwrap();
    let a = b
        .get_address("addr:1")
        .await
        .unwrap()
        .expect("address exists");
    assert_eq!(a.address, "addr:1");
    assert_eq!(a.description.as_deref(), Some("desc"));
    assert_eq!(a.scope.as_deref(), Some("scope:x"));
    assert_eq!(a.tags.as_deref(), Some("t1,t2"));
    assert_eq!(a.status, "active");

    // ensure with a None description must not clobber the existing one.
    b.ensure_address("addr:1", None, None, None).await.unwrap();
    let a = b.get_address("addr:1").await.unwrap().unwrap();
    assert_eq!(
        a.description.as_deref(),
        Some("desc"),
        "ensure_address must not overwrite descriptive fields with NULL"
    );
    // ensure with a new description updates it.
    b.ensure_address("addr:1", Some("desc2"), None, None)
        .await
        .unwrap();
    assert_eq!(
        b.get_address("addr:1")
            .await
            .unwrap()
            .unwrap()
            .description
            .as_deref(),
        Some("desc2")
    );

    // A second address in a different scope.
    b.ensure_address("addr:2", None, Some("scope:y"), None)
        .await
        .unwrap();

    // Scope filtering.
    let in_x = b.list_addresses(Some("scope:x"), false).await.unwrap();
    assert_eq!(in_x.len(), 1);
    assert_eq!(in_x[0].address, "addr:1");

    let all = b.list_addresses(None, false).await.unwrap();
    assert_eq!(all.len(), 2);

    // Status transition: active -> retired; retired drops from the default listing.
    assert!(b.set_address_status("addr:1", "retired").await.unwrap());
    assert_eq!(
        b.get_address("addr:1").await.unwrap().unwrap().status,
        "retired"
    );
    let default_list = b.list_addresses(None, false).await.unwrap();
    assert_eq!(default_list.len(), 1, "retired drops from default listing");
    assert_eq!(default_list[0].address, "addr:2");
    let with_retired = b.list_addresses(None, true).await.unwrap();
    assert_eq!(with_retired.len(), 2, "include_retired surfaces retired");

    // Status update on a missing address reports no change.
    assert!(!b.set_address_status("addr:nope", "retired").await.unwrap());
}

/// Leases/liveness: claim, exclusivity, heartbeat refresh, stale reclaim, release, and an
/// honest occupancy TTL window.
async fn leases_liveness(store: Store) {
    let b = store.connect().await;
    let addr = "lease:1";

    // Claim a free address.
    assert!(matches!(
        b.claim_lease(&new_claim(addr, "A"), 15).await.unwrap(),
        LeaseOutcome::Claimed
    ));
    let l1 = b.get_lease(addr).await.unwrap().expect("lease exists");
    assert_eq!(l1.occupant.as_deref(), Some("A"));
    let since1 = l1.since_ms;

    // Exclusivity: a second live occupant is rejected, and the current occupant is reported.
    match b.claim_lease(&new_claim(addr, "B"), 15).await.unwrap() {
        LeaseOutcome::AlreadyOccupied(cur) => {
            assert_eq!(
                cur.occupant.as_deref(),
                Some("A"),
                "current occupant reported"
            )
        }
        LeaseOutcome::Claimed => panic!("a live address must not be reclaimable by another"),
    }

    // Same-occupant re-claim is a heartbeat refresh: since_ms is preserved, heartbeat moves.
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(matches!(
        b.claim_lease(&new_claim(addr, "A"), 15).await.unwrap(),
        LeaseOutcome::Claimed
    ));
    let l2 = b.get_lease(addr).await.unwrap().unwrap();
    assert_eq!(l2.since_ms, since1, "since_ms is stable across refresh");
    assert!(l2.heartbeat_at_ms >= l1.heartbeat_at_ms);

    // Explicit heartbeat keeps the lease live.
    b.heartbeat(addr).await.unwrap();
    let occ = b.occupancy(addr, 15).await.unwrap();
    assert!(occ.occupied);
    assert_eq!(occ.occupant.as_deref(), Some("A"));

    // Occupancy honesty: a zero-width window is never live.
    assert!(
        !b.occupancy(addr, 0).await.unwrap().occupied,
        "a zero-second window can never read as occupied"
    );

    // Stale lease past the TTL window is reclaimable.
    let stale = "lease:2";
    assert!(matches!(
        b.claim_lease(&new_claim(stale, "A"), 1).await.unwrap(),
        LeaseOutcome::Claimed
    ));
    tokio::time::sleep(Duration::from_millis(1300)).await;
    assert!(
        !b.occupancy(stale, 1).await.unwrap().occupied,
        "a lease past its TTL window must read as unoccupied"
    );
    match b.claim_lease(&new_claim(stale, "B"), 1).await.unwrap() {
        LeaseOutcome::Claimed => {}
        LeaseOutcome::AlreadyOccupied(_) => panic!("a stale lease must be reclaimable"),
    }
    assert_eq!(
        b.get_lease(stale)
            .await
            .unwrap()
            .unwrap()
            .occupant
            .as_deref(),
        Some("B")
    );

    // Release frees the address.
    assert!(b.release_lease(stale, "B").await.unwrap());
    assert!(b.get_lease(stale).await.unwrap().is_none());
    assert!(!b.occupancy(stale, 1).await.unwrap().occupied);

    // Releasing a non-existent lease reports no change.
    assert!(!b.release_lease("lease:none", "X").await.unwrap());
}

/// Cursor delivery: `max_id`/`fetch_after` return rows strictly after the cursor in
/// monotonic id order, scoped to one recipient address.
async fn cursor_delivery(store: Store) {
    let b = store.connect().await;
    let addr = "cur:1";

    let m1 = b.insert_message(&new_msg(addr)).await.unwrap();
    let m2 = b.insert_message(&new_msg(addr)).await.unwrap();
    let m3 = b.insert_message(&new_msg(addr)).await.unwrap();
    assert!(m1.id < m2.id && m2.id < m3.id, "ids are monotonic");

    // A message to another address must not bleed into this cursor.
    b.insert_message(&new_msg("cur:other")).await.unwrap();

    assert_eq!(b.max_id(addr).await.unwrap(), m3.id);

    let from_zero = b.fetch_after(addr, 0).await.unwrap();
    assert_eq!(
        from_zero.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![m1.id, m2.id, m3.id]
    );

    let after_first = b.fetch_after(addr, m1.id).await.unwrap();
    assert_eq!(
        after_first.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![m2.id, m3.id],
        "fetch_after returns rows strictly after the cursor"
    );

    assert!(
        b.fetch_after(addr, m3.id).await.unwrap().is_empty(),
        "no rows beyond the head"
    );
}

/// Messages/threading: insert returns id/thread_id; a root threads itself; a reply inherits
/// the parent's thread_id; `thread_messages` returns the whole thread ordered.
async fn messages_threading(store: Store) {
    let b = store.connect().await;
    let addr = "thr:1";

    let root = b.insert_message(&new_msg(addr)).await.unwrap();
    assert_eq!(root.thread_id, root.id, "a root message threads itself");
    assert!(root.parent_id.is_none());

    let mut r1 = new_msg(addr);
    r1.parent_id = Some(root.id);
    let reply1 = b.insert_message(&r1).await.unwrap();
    assert_eq!(
        reply1.thread_id, root.id,
        "a reply inherits the parent's thread_id"
    );
    assert_eq!(reply1.parent_id, Some(root.id));

    // A nested reply stays in the original thread.
    let mut r2 = new_msg(addr);
    r2.parent_id = Some(reply1.id);
    let reply2 = b.insert_message(&r2).await.unwrap();
    assert_eq!(
        reply2.thread_id, root.id,
        "nested replies keep the root thread"
    );

    let thread = b.thread_messages(root.id).await.unwrap();
    assert_eq!(
        thread.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![root.id, reply1.id, reply2.id],
        "thread_messages returns the whole thread ordered by id"
    );

    assert!(b.get_message(root.id).await.unwrap().is_some());
    assert!(b.get_message(9_999_999).await.unwrap().is_none());
}

/// Inbox derivation: actionable = requires-disposition AND latest disposition not terminal;
/// `--all` includes non-actionable; terminal dispositions drop a message, non-terminal keep it.
async fn inbox_derivation(store: Store) {
    let b = store.connect().await;
    let addr = "inbox:1";

    let mut needs = new_msg(addr);
    needs.requires_disposition = true;
    let m1 = b.insert_message(&needs).await.unwrap();

    // requires_disposition = false -> never actionable.
    let m2 = b.insert_message(&new_msg(addr)).await.unwrap();

    let actionable = b.inbox(addr, false, 50).await.unwrap();
    assert_eq!(actionable.len(), 1);
    assert_eq!(actionable[0].message.id, m1.id);
    assert!(actionable[0].actionable);

    let all = b.inbox(addr, true, 50).await.unwrap();
    assert_eq!(all.len(), 2, "--all includes non-actionable messages");
    let m2_item = all.iter().find(|it| it.message.id == m2.id).unwrap();
    assert!(!m2_item.actionable);

    // A terminal disposition drops the message from the actionable set.
    b.insert_disposition(m1.id, addr, "handled", None, None)
        .await
        .unwrap();
    assert!(
        b.inbox(addr, false, 50).await.unwrap().is_empty(),
        "terminal disposition removes a message from the actionable inbox"
    );

    // Non-terminal dispositions keep the message actionable.
    let mut needs3 = new_msg(addr);
    needs3.requires_disposition = true;
    let m3 = b.insert_message(&needs3).await.unwrap();
    for state in ["acknowledged", "deferred", "escalated"] {
        b.insert_disposition(m3.id, addr, state, None, None)
            .await
            .unwrap();
        let inbox = b.inbox(addr, false, 50).await.unwrap();
        assert_eq!(
            inbox.len(),
            1,
            "non-terminal disposition '{state}' keeps the message actionable"
        );
        assert_eq!(inbox[0].message.id, m3.id);
        assert_eq!(inbox[0].latest_disposition.as_deref(), Some(state));
    }

    // A terminal disposition then drops it.
    b.insert_disposition(m3.id, addr, "closed", None, None)
        .await
        .unwrap();
    assert!(b.inbox(addr, false, 50).await.unwrap().is_empty());
}

/// Dispositions: insert; latest-per-(message, recipient) wins; terminality semantics; and a
/// disposition for a different recipient does not affect this recipient's inbox.
async fn dispositions(store: Store) {
    let b = store.connect().await;
    let addr = "disp:1";

    let mut needs = new_msg(addr);
    needs.requires_disposition = true;
    let m = b.insert_message(&needs).await.unwrap();

    let d1 = b
        .insert_disposition(m.id, addr, "acknowledged", Some("ack note"), Some("me"))
        .await
        .unwrap();
    let d2 = b
        .insert_disposition(m.id, addr, "handled", None, None)
        .await
        .unwrap();
    assert!(d1.id < d2.id);

    let all = b.dispositions_for(m.id).await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].state, "acknowledged");
    assert_eq!(all[0].note.as_deref(), Some("ack note"));
    assert_eq!(all[0].by_principal.as_deref(), Some("me"));
    assert_eq!(all[1].state, "handled");

    // Latest-per-recipient wins: the terminal "handled" is the latest, so it drops out.
    assert!(b.inbox(addr, false, 50).await.unwrap().is_empty());

    // A disposition recorded for a *different* recipient must not affect this recipient.
    let mut needs2 = new_msg(addr);
    needs2.requires_disposition = true;
    let m2 = b.insert_message(&needs2).await.unwrap();
    b.insert_disposition(m2.id, "someone:else", "handled", None, None)
        .await
        .unwrap();
    let inbox = b.inbox(addr, false, 50).await.unwrap();
    assert_eq!(
        inbox.len(),
        1,
        "another recipient's disposition does not resolve this recipient's message"
    );
    assert_eq!(inbox[0].message.id, m2.id);
    assert!(inbox[0].latest_disposition.is_none());
}

/// Export: address (to or from), thread, and since-cursor filters.
async fn export_filters(store: Store) {
    let b = store.connect().await;

    // From e:1 -> e:2.
    let mut m1 = new_msg("e:2");
    m1.from_addr = Some("e:1".to_string());
    let a = b.insert_message(&m1).await.unwrap();
    // To e:1.
    let bmsg = b.insert_message(&new_msg("e:1")).await.unwrap();
    // A reply on a's thread, unrelated to e:1.
    let mut m3 = new_msg("e:3");
    m3.parent_id = Some(a.id);
    let c = b.insert_message(&m3).await.unwrap();
    // Wholly unrelated.
    let d = b.insert_message(&new_msg("e:9")).await.unwrap();

    let all = b.export(None, None, 0).await.unwrap();
    assert_eq!(all.len(), 4);

    // Address filter matches to_addr OR from_addr.
    let by_addr = b.export(Some("e:1"), None, 0).await.unwrap();
    let ids: HashSet<i64> = by_addr.iter().map(|m| m.id).collect();
    assert_eq!(by_addr.len(), 2, "address filter matches to OR from");
    assert!(ids.contains(&a.id) && ids.contains(&bmsg.id));
    assert!(!ids.contains(&d.id));

    // Thread filter returns the root and its replies, ordered.
    let by_thread = b.export(None, Some(a.id), 0).await.unwrap();
    assert_eq!(
        by_thread.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![a.id, c.id]
    );

    // Since cursor returns rows strictly after the given id.
    let since = b.export(None, None, a.id).await.unwrap();
    assert_eq!(since.len(), 3);
    assert!(since.iter().all(|m| m.id > a.id));
}

/// Concurrency: multiple independent connections inserting concurrently produce distinct,
/// monotonic ids with no lost writes (the cursor model depends on this).
async fn concurrency(store: Store) {
    let addr = "conc:1";
    const WRITERS: usize = 4;
    const PER_WRITER: usize = 10;

    // Establish the store (file/schema) before writers attach, as in real use where a store
    // already exists before independent processes connect to it.
    let _ = store.connect().await;

    let mut handles = Vec::new();
    for _ in 0..WRITERS {
        let store = store.clone();
        handles.push(tokio::spawn(async move {
            let b = store.connect().await;
            for _ in 0..PER_WRITER {
                b.insert_message(&new_msg(addr)).await.unwrap();
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    let b = store.connect().await;
    let rows = b.fetch_after(addr, 0).await.unwrap();
    let total = WRITERS * PER_WRITER;
    assert_eq!(rows.len(), total, "no lost writes under concurrency");

    let ids: Vec<i64> = rows.iter().map(|m| m.id).collect();
    let unique: HashSet<i64> = ids.iter().copied().collect();
    assert_eq!(unique.len(), total, "ids are distinct under concurrency");
    assert!(
        ids.windows(2).all(|w| w[0] < w[1]),
        "ids are strictly monotonic in cursor order"
    );
    assert_eq!(b.max_id(addr).await.unwrap(), *ids.last().unwrap());
}

// ----------------------------------------------------------------------------------------
// SQLite fixture (always runs)
// ----------------------------------------------------------------------------------------

#[cfg(feature = "sqlite")]
mod sqlite_fixture {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_temp_root() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "telex-conformance-{}-{}",
            std::process::id(),
            now_ms()
        ));
        p
    }

    fn make_store(root: PathBuf, counter: Arc<AtomicU64>) -> Store {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        let path = root
            .join(format!("conf-{n}.db"))
            .to_string_lossy()
            .to_string();
        Store {
            connect: Arc::new(move || {
                let path = path.clone();
                Box::pin(async move {
                    let b = telex::backend::sqlite::SqliteBackend::open(&path)
                        .expect("open sqlite backend");
                    b.init_schema().await.expect("init sqlite schema");
                    Arc::new(b) as Arc<dyn Backend>
                })
            }),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn sqlite_conformance() {
        let root = unique_temp_root();
        std::fs::create_dir_all(&root).unwrap();
        let counter = Arc::new(AtomicU64::new(0));

        {
            let root = root.clone();
            run_all(move || {
                let root = root.clone();
                let counter = counter.clone();
                Box::pin(async move { make_store(root, counter) })
            })
            .await;
        }

        std::fs::remove_dir_all(&root).ok();
    }
}

// ----------------------------------------------------------------------------------------
// Postgres fixture (runs only when TELEX_PG_URL is set; skips cleanly otherwise)
// ----------------------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod postgres_fixture {
    use super::*;
    use telex::backend::postgres::{make_tls, sanitize_ident, PgBackend};

    /// Run a statement on a short-lived admin connection (search_path = public), so schema
    /// DDL and `TRUNCATE` can use schema-qualified names regardless of the connection state.
    async fn admin_exec(cfg: &tokio_postgres::Config, sql: &str) -> anyhow::Result<()> {
        let (client, connection) = cfg.connect(make_tls()?).await?;
        let handle = tokio::spawn(async move {
            let _ = connection.await;
        });
        let res = client.batch_execute(sql).await;
        drop(client);
        let _ = handle.await;
        res?;
        Ok(())
    }

    fn make_store(cfg: tokio_postgres::Config, schema: String) -> BoxFut<Store> {
        Box::pin(async move {
            // Create the schema + tables, then truncate to guarantee an empty store.
            let b = PgBackend::connect_with(cfg.clone(), Some(&schema))
                .await
                .expect("connect postgres");
            b.init_schema().await.expect("init postgres schema");
            admin_exec(
                &cfg,
                &format!(
                    "TRUNCATE {schema}.addresses, {schema}.leases, {schema}.messages, \
                     {schema}.dispositions RESTART IDENTITY"
                ),
            )
            .await
            .expect("truncate schema");

            let cfg2 = cfg.clone();
            let schema2 = schema.clone();
            Store {
                connect: Arc::new(move || {
                    let cfg2 = cfg2.clone();
                    let schema2 = schema2.clone();
                    Box::pin(async move {
                        let b = PgBackend::connect_with(cfg2, Some(&schema2))
                            .await
                            .expect("connect postgres");
                        Arc::new(b) as Arc<dyn Backend>
                    })
                }),
            }
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn postgres_conformance() {
        let url = match std::env::var("TELEX_PG_URL") {
            Ok(u) if !u.trim().is_empty() => u,
            _ => {
                eprintln!(
                    "[conformance] TELEX_PG_URL not set; skipping the Postgres conformance suite."
                );
                return;
            }
        };

        let mut cfg: tokio_postgres::Config = url
            .parse()
            .expect("TELEX_PG_URL must be a libpq URI or key=value DSN");
        if let Ok(pw) = std::env::var("TELEX_PG_PASSWORD") {
            if !pw.is_empty() {
                cfg.password(pw);
            }
        }

        let base = std::env::var("TELEX_PG_SCHEMA").unwrap_or_else(|_| "telex_conformance".into());
        let schema = sanitize_ident(&format!("{base}_{}_{}", std::process::id(), now_ms()))
            .expect("derived schema name must be a valid identifier");

        // Drop any leftover, run the battery against a fresh schema, then drop it.
        admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
            .await
            .expect("pre-run schema drop");

        let cfg_run = cfg.clone();
        let schema_run = schema.clone();
        run_all(move || make_store(cfg_run.clone(), schema_run.clone())).await;

        admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
            .await
            .expect("post-run schema drop");
    }
}
