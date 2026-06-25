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
    // Coverage checklist — every `Backend` trait method must be exercised by at least one
    // scenario here; when the trait grows, add the method + a scenario so the battery fails
    // loudly instead of silently leaving new contract surface untested:
    //   kind / capabilities / notify_new ........ capabilities_and_signals
    //   init_schema ............................. schema_idempotent
    //   ensure_address / get_address /
    //     set_address_status / list_addresses ... addresses_directory
    //   claim_lease / heartbeat / release_lease /
    //     get_lease / occupancy ................. leases_liveness
    //   fetch_undelivered ....................... undelivered_delivery, delivery_backlog, concurrency
    //   mark_delivered .......................... undelivered_delivery, delivery_backlog
    //   insert_message / get_message /
    //     thread_messages ....................... messages_threading
    //   inbox ................................... inbox_derivation
    //   insert_disposition / dispositions_for ... dispositions, inbox_derivation
    //   export .................................. export_filters
    capabilities_and_signals(make_store().await).await;
    schema_idempotent(make_store().await).await;
    addresses_directory(make_store().await).await;
    leases_liveness(make_store().await).await;
    undelivered_delivery(make_store().await).await;
    delivery_backlog(make_store().await).await;
    messages_threading(make_store().await).await;
    inbox_derivation(make_store().await).await;
    dispositions(make_store().await).await;
    export_filters(make_store().await).await;
    concurrency(make_store().await).await;
}

/// Capabilities + signals smoke coverage: `kind`, `capabilities`, and the best-effort
/// `notify_new` must be callable and self-consistent, so a new backend can't leave the
/// metadata/notify surface silently untested.
async fn capabilities_and_signals(store: Store) {
    let b = store.connect().await;

    assert!(
        !b.kind().is_empty(),
        "kind() must be a non-empty backend name"
    );
    let caps = b.capabilities();
    assert!(
        !caps.push.is_empty(),
        "capabilities.push must describe a delivery mechanism"
    );
    assert!(
        !caps.lease.is_empty(),
        "capabilities.lease must describe a liveness mechanism"
    );

    // notify_new is a best-effort signal (a no-op where push is unsupported); it must never
    // error, even for an address with no occupant or messages.
    b.notify_new("sig:1", 1, now_ms())
        .await
        .expect("notify_new must be a safe best-effort signal");
}

/// `init_schema` must be safe to call repeatedly *and non-destructive*: re-initialising a
/// store that already holds data must preserve it (a backend that drops/recreates tables on
/// init would corrupt a live store and must fail here).
async fn schema_idempotent(store: Store) {
    let b = store.connect().await;

    // Seed one of each persisted entity (connect already ran init_schema once).
    b.ensure_address("idem:1", Some("d"), None, None)
        .await
        .unwrap();
    let mut needs = new_msg("idem:1");
    needs.requires_disposition = true;
    let m = b.insert_message(&needs).await.unwrap();
    b.insert_disposition(m.id, "idem:1", "acknowledged", None, None)
        .await
        .unwrap();
    b.claim_lease(&new_claim("idem:1", "A"), 15).await.unwrap();

    // Re-initialising repeatedly must be a no-op that preserves existing data.
    for _ in 0..3 {
        b.init_schema()
            .await
            .expect("init_schema must be idempotent");
    }

    assert!(
        b.get_address("idem:1").await.unwrap().is_some(),
        "init_schema must not drop existing addresses"
    );
    assert!(
        b.get_message(m.id).await.unwrap().is_some(),
        "init_schema must not drop existing messages"
    );
    assert_eq!(
        b.dispositions_for(m.id).await.unwrap().len(),
        1,
        "init_schema must not drop existing dispositions"
    );
    assert!(
        b.get_lease("idem:1").await.unwrap().is_some(),
        "init_schema must not drop existing leases"
    );
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

    // Authorization: a non-holder cannot release someone else's lease (no lease theft).
    assert!(
        !b.release_lease(addr, "B").await.unwrap(),
        "release_lease must reject a non-holder"
    );
    assert_eq!(
        b.get_lease(addr)
            .await
            .unwrap()
            .unwrap()
            .occupant
            .as_deref(),
        Some("A"),
        "a rejected release leaves the original holder in place"
    );

    // Same-occupant re-claim is a heartbeat refresh: since_ms is preserved, heartbeat moves.
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(matches!(
        b.claim_lease(&new_claim(addr, "A"), 15).await.unwrap(),
        LeaseOutcome::Claimed
    ));
    let l2 = b.get_lease(addr).await.unwrap().unwrap();
    assert_eq!(l2.since_ms, since1, "since_ms is stable across refresh");
    assert!(
        l2.heartbeat_at_ms > l1.heartbeat_at_ms,
        "a same-occupant re-claim must advance heartbeat_at_ms"
    );

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

    // Release clears occupant fields but retains the row (epoch preserved for monotonicity).
    assert!(b.release_lease(stale, "B").await.unwrap());
    // Row persists; occupancy is now false.
    assert!(!b.occupancy(stale, 1).await.unwrap().occupied);

    // Releasing a non-existent lease reports no change.
    assert!(!b.release_lease("lease:none", "X").await.unwrap());
}

/// Undelivered delivery: `fetch_undelivered` returns every message addressed here that has no
/// delivery record and a non-terminal disposition, in id order, scoped to one recipient — and
/// crucially returns a lower undelivered id even after a HIGHER id has been delivered. That last
/// property is what closes the Postgres commit-order gap (issue #18): visibility no longer depends
/// on a monotonic id cursor, so a concurrently-committed lower id is never skipped.
async fn undelivered_delivery(store: Store) {
    let b = store.connect().await;
    let addr = "und:1";

    let m1 = b.insert_message(&new_msg(addr)).await.unwrap();
    let m2 = b.insert_message(&new_msg(addr)).await.unwrap();
    let m3 = b.insert_message(&new_msg(addr)).await.unwrap();
    assert!(m1.id < m2.id && m2.id < m3.id, "ids are monotonic");

    // A message to another address must not bleed into this recipient's undelivered set.
    b.insert_message(&new_msg("und:other")).await.unwrap();

    let all = b.fetch_undelivered(addr).await.unwrap();
    assert_eq!(
        all.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![m1.id, m2.id, m3.id],
        "all undelivered messages for the address, ordered by id, nothing from other addresses"
    );

    // The gap-closing invariant: deliver the HIGHER ids m2 and m3; the lower undelivered m1 must
    // still be returned. A high-water cursor parked at m3 would skip m1 — `fetch_undelivered` does
    // not, because delivery state (not id ordering) decides visibility.
    b.mark_delivered(m2.id, addr, Some("holderA"))
        .await
        .unwrap();
    b.mark_delivered(m3.id, addr, Some("holderA"))
        .await
        .unwrap();
    assert_eq!(
        b.fetch_undelivered(addr)
            .await
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect::<Vec<_>>(),
        vec![m1.id],
        "a lower undelivered id survives delivery of higher ids (issue #18 invariant)"
    );

    // Delivering the last one empties the set.
    b.mark_delivered(m1.id, addr, Some("holderA"))
        .await
        .unwrap();
    assert!(
        b.fetch_undelivered(addr).await.unwrap().is_empty(),
        "no undelivered messages remain once all are delivered"
    );
}

/// Delivery + disposition interplay: `mark_delivered` records a holder->waiter handoff, and
/// `fetch_undelivered` excludes a message once it is delivered OR terminally dispositioned, in id
/// order — the two orthogonal do-not-deliver signals (a delivery record, primary; a terminal
/// disposition, secondary for out-of-band `inbox` recovery), with the delivery record dominating.
async fn delivery_backlog(store: Store) {
    let b = store.connect().await;
    let addr = "backlog:1";

    // Three messages queued while the address was unoccupied (no holder ever delivered them).
    let m1 = b.insert_message(&new_msg(addr)).await.unwrap();
    let m2 = b.insert_message(&new_msg(addr)).await.unwrap();
    let m3 = b.insert_message(&new_msg(addr)).await.unwrap();
    // A message to another address must never leak into this address's undelivered set.
    let other = b.insert_message(&new_msg("backlog:other")).await.unwrap();

    // All three are undelivered and undispositioned -> the full set, ordered by id.
    let bl = b.fetch_undelivered(addr).await.unwrap();
    assert_eq!(
        bl.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![m1.id, m2.id, m3.id],
        "undelivered, non-terminal messages, ordered by id"
    );
    assert!(
        !bl.iter().any(|m| m.id == other.id),
        "another address's messages never appear here"
    );

    // Delivering m1 (the holder->waiter handoff) drops it from the undelivered set: not redelivered.
    b.mark_delivered(m1.id, addr, Some("holderA"))
        .await
        .unwrap();
    let after = b.fetch_undelivered(addr).await.unwrap();
    assert_eq!(
        after.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![m2.id, m3.id],
        "a delivered message is not re-surfaced as undelivered"
    );

    // A terminal disposition on a never-delivered message also removes it — covers out-of-band
    // recovery via `inbox` + manual disposition without the message ever passing through `wait`.
    b.insert_disposition(m2.id, addr, "handled", None, None)
        .await
        .unwrap();
    let after2 = b.fetch_undelivered(addr).await.unwrap();
    assert_eq!(
        after2.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![m3.id],
        "a terminally dispositioned message is excluded"
    );

    // A non-terminal disposition keeps a never-delivered message in the undelivered set.
    b.insert_disposition(m3.id, addr, "acknowledged", None, None)
        .await
        .unwrap();
    assert_eq!(
        b.fetch_undelivered(addr)
            .await
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect::<Vec<_>>(),
        vec![m3.id],
        "a non-terminal disposition (acknowledged) does not exclude a message"
    );

    // Two-signal interaction: a *delivered* message that is later terminally dispositioned and then
    // reopened (latest disposition non-terminal) must STILL stay excluded. The durable delivery
    // record dominates the disposition state, so a reopen never resurrects an already-delivered
    // message — distinguishing this from inbox's latest-disposition-wins rule.
    b.mark_delivered(m3.id, addr, Some("holderA"))
        .await
        .unwrap();
    b.insert_disposition(m3.id, addr, "closed", None, None)
        .await
        .unwrap();
    b.insert_disposition(m3.id, addr, "acknowledged", None, None)
        .await
        .unwrap();
    assert!(
        b.fetch_undelivered(addr).await.unwrap().is_empty(),
        "a delivered message stays excluded even when reopened (delivery record dominates)"
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

    // Reopen: a *later* non-terminal disposition makes the message actionable again. This
    // distinguishes "latest-per-(message,recipient) wins" from a weaker "any terminal
    // disposition exists" rule — a backend implementing the latter would wrongly keep it gone.
    b.insert_disposition(m3.id, addr, "acknowledged", None, None)
        .await
        .unwrap();
    let reopened = b.inbox(addr, false, 50).await.unwrap();
    assert_eq!(
        reopened.len(),
        1,
        "a later non-terminal disposition reopens the message (latest wins, not any-terminal)"
    );
    assert_eq!(reopened[0].message.id, m3.id);
    assert_eq!(
        reopened[0].latest_disposition.as_deref(),
        Some("acknowledged")
    );
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

/// Concurrency: multiple independent connections inserting concurrently each receive a
/// distinct id with no lost writes. Each writer records the ids it was handed, so the assertions
/// exercise concurrent id *assignment* rather than merely restating `fetch_undelivered`'s
/// `ORDER BY id`.
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
            let mut mine = Vec::with_capacity(PER_WRITER);
            for _ in 0..PER_WRITER {
                mine.push(b.insert_message(&new_msg(addr)).await.unwrap().id);
            }
            mine
        }));
    }
    let mut assigned: Vec<i64> = Vec::new();
    for h in handles {
        assigned.extend(h.await.unwrap());
    }

    let total = WRITERS * PER_WRITER;
    // Every insert across every connection was handed a distinct id (no collisions).
    let assigned_set: HashSet<i64> = assigned.iter().copied().collect();
    assert_eq!(
        assigned_set.len(),
        total,
        "concurrent inserts must each receive a distinct id"
    );

    // The store holds exactly the ids handed back to the writers — no lost writes, none extra.
    // None are delivered, so the undelivered set is the full set of stored ids.
    let conn = store.connect().await;
    let rows = conn.fetch_undelivered(addr).await.unwrap();
    let stored: HashSet<i64> = rows.iter().map(|m| m.id).collect();
    assert_eq!(
        stored, assigned_set,
        "persisted ids must be exactly the ids handed back to the writers (no lost writes)"
    );
    assert_eq!(
        rows.iter().map(|m| m.id).max().unwrap(),
        *assigned_set.iter().max().unwrap(),
        "the highest persisted id matches the highest assigned id"
    );
}

// ----------------------------------------------------------------------------------------
// Fixture harness
// ----------------------------------------------------------------------------------------

/// Run the full battery, then run `cleanup` whether the battery succeeded *or panicked*, and
/// finally re-raise any panic so the test still fails. Centralises the panic-safe teardown so
/// each fixture only supplies its own store factory and cleanup action.
async fn run_with_cleanup<F, C, CFut>(make_store: F, cleanup: C)
where
    F: Fn() -> BoxFut<Store> + Send + 'static,
    C: FnOnce() -> CFut,
    CFut: Future<Output = ()>,
{
    let result = tokio::spawn(async move { run_all(make_store).await }).await;
    cleanup().await;
    if let Err(e) = result {
        std::panic::resume_unwind(e.into_panic());
    }
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

        let run_root = root.clone();
        run_with_cleanup(
            move || {
                let root = run_root.clone();
                let counter = counter.clone();
                Box::pin(async move { make_store(root, counter) })
            },
            move || async move {
                std::fs::remove_dir_all(&root).ok();
            },
        )
        .await;
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
                     {schema}.dispositions, {schema}.deliveries RESTART IDENTITY"
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
        // `TELEX_PG_REQUIRE=1` turns a missing/empty `TELEX_PG_URL` into a failure instead of a
        // silent skip, so a CI job that means to exercise the Postgres leg can't pass by
        // accidentally skipping it.
        let require = std::env::var("TELEX_PG_REQUIRE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let url = match std::env::var("TELEX_PG_URL") {
            Ok(u) if !u.trim().is_empty() => u,
            _ => {
                assert!(
                    !require,
                    "TELEX_PG_REQUIRE is set but TELEX_PG_URL is unset/empty; \
                     refusing to skip the Postgres conformance suite."
                );
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

        let base = sanitize_ident(
            &std::env::var("TELEX_PG_SCHEMA").unwrap_or_else(|_| "telex_conformance".into()),
        )
        .expect("TELEX_PG_SCHEMA must be a valid identifier");
        let schema = sanitize_ident(&format!("{base}_{}_{}", std::process::id(), now_ms()))
            .expect("derived schema name must be a valid identifier");

        // Reclaim schemas leaked by a previously *hard-killed* run without ever touching a live
        // one. Match only the exact per-run shape `^{base}_<pid>_<ms>$` AND require the embedded
        // creation timestamp to be older than a generous cutoff, so a concurrently running
        // suite's recent schema is never dropped; the active schema is excluded too. Unrelated
        // operator schemas don't match the shape and are left alone. (`base` is a sanitised
        // identifier, so it is safe to interpolate into the regex and literal.)
        let cutoff_ms = now_ms() - 3_600_000; // 1 hour
        admin_exec(
            &cfg,
            &format!(
                "DO $$ DECLARE s text; ts bigint; BEGIN \
                   FOR s IN SELECT schema_name FROM information_schema.schemata \
                            WHERE schema_name ~ '^{base}_[0-9]+_[0-9]+$' \
                   LOOP ts := substring(s from '_([0-9]+)$')::bigint; \
                     IF ts < {cutoff_ms} AND s <> '{schema}' THEN \
                       EXECUTE format('DROP SCHEMA IF EXISTS %I CASCADE', s); \
                     END IF; \
                   END LOOP; END $$;"
            ),
        )
        .await
        .expect("pre-run leftover sweep");

        let cfg_run = cfg.clone();
        let schema_run = schema.clone();
        let cfg_cleanup = cfg.clone();
        let schema_cleanup = schema.clone();
        run_with_cleanup(
            move || make_store(cfg_run.clone(), schema_run.clone()),
            move || async move {
                admin_exec(
                    &cfg_cleanup,
                    &format!("DROP SCHEMA IF EXISTS {schema_cleanup} CASCADE"),
                )
                .await
                .expect("post-run schema drop");
            },
        )
        .await;
    }

    /// Issue #18, against real Postgres MVCC: a lower id committed AFTER a higher id (reverse commit
    /// order) must still be delivered by the LIVE holder with no restart. Two independent
    /// connections insert to one address; connection A holds the LOWER id in an open transaction
    /// while connection B commits the HIGHER id. After the higher id is delivered and A finally
    /// commits, `fetch_undelivered` must return the lower id — whereas the old high-water cursor
    /// (`WHERE id > <delivered>`) would skip it. This is the faithful reproduction the conformance
    /// battery (which inserts via auto-committing `insert_message`) cannot express. Gated on
    /// `TELEX_PG_URL`; uses a distinct `telex_issue18_*` schema so it never collides with the
    /// conformance schema.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn postgres_out_of_order_commit_delivers_lower_id() {
        let require = std::env::var("TELEX_PG_REQUIRE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let url = match std::env::var("TELEX_PG_URL") {
            Ok(u) if !u.trim().is_empty() => u,
            _ => {
                assert!(
                    !require,
                    "TELEX_PG_REQUIRE is set but TELEX_PG_URL is unset/empty; \
                     refusing to skip the issue-#18 out-of-order-commit test."
                );
                eprintln!(
                    "[conformance] TELEX_PG_URL not set; skipping the issue-#18 out-of-order test."
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
        let schema = sanitize_ident(&format!(
            "telex_issue18_{}_{}",
            std::process::id(),
            now_ms()
        ))
        .expect("derived schema name must be a valid identifier");

        // Create the schema + tables via the backend, then run the body with panic-safe cleanup.
        let b = PgBackend::connect_with(cfg.clone(), Some(&schema))
            .await
            .expect("connect postgres");
        b.init_schema().await.expect("init postgres schema");
        drop(b);

        let cfg_body = cfg.clone();
        let schema_body = schema.clone();
        let result = tokio::spawn(async move {
            out_of_order_commit_body(cfg_body, &schema_body).await;
        })
        .await;

        admin_exec(&cfg, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
            .await
            .expect("post-test schema drop");
        if let Err(e) = result {
            std::panic::resume_unwind(e.into_panic());
        }
    }

    /// Open two independent connections, force reverse-id commit order, and assert
    /// `fetch_undelivered` sees the late lower id while a raw `id > <delivered>` cursor does not.
    async fn out_of_order_commit_body(cfg: tokio_postgres::Config, schema: &str) {
        let addr = "reorder:pg";

        // Connection A: BEGIN; insert the LOWER id; hold the transaction open (uncommitted, so the
        // row is invisible to every other connection).
        let (ca, ca_conn) = cfg
            .connect(make_tls().expect("tls"))
            .await
            .expect("connect A");
        let ca_handle = tokio::spawn(async move {
            let _ = ca_conn.await;
        });
        ca.batch_execute(&format!("SET search_path TO {schema}; BEGIN"))
            .await
            .unwrap();
        let id_lower: i64 = ca
            .query_one(
                "INSERT INTO messages(to_addr, body, sent_at_ms, created_at_ms) \
                 VALUES ($1,'lower',0,0) RETURNING id",
                &[&addr],
            )
            .await
            .unwrap()
            .get("id");

        // Connection B (auto-commit): insert the HIGHER id and commit it immediately.
        let (cb, cb_conn) = cfg
            .connect(make_tls().expect("tls"))
            .await
            .expect("connect B");
        let cb_handle = tokio::spawn(async move {
            let _ = cb_conn.await;
        });
        cb.batch_execute(&format!("SET search_path TO {schema}"))
            .await
            .unwrap();
        let id_higher: i64 = cb
            .query_one(
                "INSERT INTO messages(to_addr, body, sent_at_ms, created_at_ms) \
                 VALUES ($1,'higher',0,0) RETURNING id",
                &[&addr],
            )
            .await
            .unwrap()
            .get("id");
        assert!(
            id_higher > id_lower,
            "B must allocate a higher id than A (got {id_higher} vs {id_lower})"
        );

        // The holder's backend connection. Only the committed higher id is visible; A's lower id is
        // still in flight.
        let backend = PgBackend::connect_with(cfg.clone(), Some(schema))
            .await
            .expect("connect backend");
        let before: Vec<i64> = backend
            .fetch_undelivered(addr)
            .await
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(
            before,
            vec![id_higher],
            "only the committed higher id is visible while A's transaction is open"
        );

        // The holder delivers the higher id (this is the moment a high-water cursor would advance
        // past it and lose the still-uncommitted lower id forever, until a restart).
        backend
            .mark_delivered(id_higher, addr, Some("holder-pg"))
            .await
            .unwrap();

        // Now A commits: the LOWER id becomes visible, committed *behind* the delivered higher id.
        ca.batch_execute("COMMIT").await.unwrap();

        // THE FIX: the live holder's drain query returns the late lower id — no restart required.
        let undelivered: Vec<i64> = backend
            .fetch_undelivered(addr)
            .await
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(
            undelivered,
            vec![id_lower],
            "fetch_undelivered returns the concurrently-committed lower id (issue #18 closed)"
        );

        // CONTRAST: the OLD high-water cursor model (`id > <delivered>`) skips it. We run that raw
        // query inline (the `fetch_after` method that did this is removed) to show it misses the
        // lower id even though it is now committed and undelivered — exactly the bug #18 fixes.
        let cursor_rows = cb
            .query(
                "SELECT id FROM messages WHERE to_addr=$1 AND id>$2 ORDER BY id",
                &[&addr, &id_higher],
            )
            .await
            .unwrap();
        assert!(
            cursor_rows.is_empty(),
            "a high-water cursor parked at the delivered id skips the lower id (the #18 bug)"
        );

        drop(ca);
        drop(cb);
        let _ = ca_handle.await;
        let _ = cb_handle.await;
    }
}

// ----------------------------------------------------------------------------------------
// SQLite epoch-aware storage tests (P1 — §11 / §13 / §17 rows 2,4,5,7,12,17,19)
// ----------------------------------------------------------------------------------------

#[cfg(feature = "sqlite")]
mod sqlite_epoch_tests {
    use super::*;
    use telex::backend::sqlite::SqliteBackend;
    use telex::model::{DeliveryOutcome, EpochClaimResult};

    fn tmp_path(label: &str) -> String {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "telex-epoch-{}-{}-{}.db",
            label,
            std::process::id(),
            now_ms()
        ));
        p.to_string_lossy().to_string()
    }

    async fn fresh(label: &str) -> (SqliteBackend, String) {
        let path = tmp_path(label);
        let b = SqliteBackend::open(&path).expect("open");
        b.init_schema().await.expect("init");
        (b, path)
    }

    /// After `release_lease`, the row must persist (epoch preserved); a second claim must get
    /// epoch+1, not epoch=1 — proving the monotonic high-water mark is never reset.
    #[tokio::test]
    async fn epoch_non_deleting_release_and_monotonicity() {
        let (b, _path) = fresh("epoch-monotone").await;
        let addr = "epoch:monotone";
        let claim = LeaseClaim {
            address: addr.to_string(),
            occupant: "A".into(),
            ..Default::default()
        };

        // First claim → epoch 1.
        b.claim_lease(&claim, 60).await.unwrap();
        let row1 = b.get_lease(addr).await.unwrap().expect("row after claim");
        assert_eq!(row1.lease_epoch, Some(1), "first claim epoch=1");

        // Release — row must survive.
        assert!(b.release_lease(addr, "A").await.unwrap());
        let row2 = b.get_lease(addr).await.unwrap().expect("row after release");
        assert_eq!(row2.lease_epoch, Some(1), "epoch preserved after release");
        assert_eq!(row2.owner_instance_id, None, "owner cleared after release");

        // Second claim (different occupant) must increment to epoch 2.
        let claim2 = LeaseClaim {
            address: addr.to_string(),
            occupant: "B".into(),
            ..Default::default()
        };
        b.claim_lease(&claim2, 60).await.unwrap();
        let row3 = b
            .get_lease(addr)
            .await
            .unwrap()
            .expect("row after re-claim");
        assert_eq!(
            row3.lease_epoch,
            Some(2),
            "second claim epoch=2 (monotonic)"
        );
    }

    /// `release_lease` must NOT delete the row (§11.2 non-deleting release).
    #[tokio::test]
    async fn epoch_release_no_row_deletion() {
        let (b, _path) = fresh("epoch-nodelete").await;
        let addr = "epoch:nodelete";
        let claim = LeaseClaim {
            address: addr.to_string(),
            occupant: "X".into(),
            ..Default::default()
        };
        b.claim_lease(&claim, 60).await.unwrap();
        assert!(b.release_lease(addr, "X").await.unwrap());
        // Row must still exist after release.
        assert!(
            b.get_lease(addr).await.unwrap().is_some(),
            "release_lease must not delete the row"
        );
        // And occupancy must be false.
        assert!(!b.occupancy(addr, 60).await.unwrap().occupied);
    }

    /// A v0 store with an existing `deliveries` table but no `consumed_at_ms` column must migrate
    /// cleanly. This exercises the guarded ALTER TABLE path.
    #[tokio::test]
    async fn legacy_deliveries_table_migrates_consumed_column() {
        let path = tmp_path("legacy-deliveries");
        {
            let conn = rusqlite::Connection::open(&path).expect("open legacy sqlite");
            conn.execute_batch(
                "CREATE TABLE deliveries (
                    id              INTEGER PRIMARY KEY AUTOINCREMENT,
                    message_id      INTEGER NOT NULL,
                    recipient       TEXT NOT NULL,
                    occupant        TEXT,
                    delivered_at_ms INTEGER NOT NULL,
                    UNIQUE(message_id, recipient)
                );
                INSERT INTO deliveries(message_id, recipient, occupant, delivered_at_ms)
                VALUES (7, 'addr:a', 'old-holder', 1234);",
            )
            .expect("seed legacy deliveries");
        }

        let b = SqliteBackend::open(&path).expect("open migrated sqlite");
        b.init_schema().await.expect("migrate schema");

        let conn = rusqlite::Connection::open(&path).expect("reopen migrated sqlite");
        let consumed_at: i64 = conn
            .query_row(
                "SELECT consumed_at_ms FROM deliveries WHERE message_id=7 AND recipient='addr:a'",
                [],
                |r| r.get(0),
            )
            .expect("consumed_at_ms backfilled");
        assert_eq!(consumed_at, 1234);
    }

    /// `heartbeat_epoch` returns `true` for current owner/epoch, `false` for stale/wrong.
    #[tokio::test]
    async fn epoch_heartbeat_staleness() {
        let (b, _path) = fresh("epoch-hb").await;
        let addr = "epoch:hb";
        let owner = "owner-hb";

        // Set up via claim_epoch_lease.
        let result = b.claim_epoch_lease(addr, owner, 15).await.unwrap();
        let epoch = match result {
            EpochClaimResult::Claimed(ref e) => e.lease_epoch,
            other => panic!("expected Claimed, got {:?}", other),
        };

        // Correct owner+epoch → returns true.
        assert!(
            b.heartbeat_epoch(addr, owner, epoch).await.unwrap(),
            "heartbeat with correct owner+epoch must return true"
        );

        // Wrong epoch → returns false.
        assert!(
            !b.heartbeat_epoch(addr, owner, epoch + 99).await.unwrap(),
            "heartbeat with wrong epoch must return false"
        );

        // Wrong owner → returns false.
        assert!(
            !b.heartbeat_epoch(addr, "impostor", epoch).await.unwrap(),
            "heartbeat with wrong owner must return false"
        );
    }

    /// `mark_consumed_if_current_owner` returns the correct `DeliveryOutcome` variant based on
    /// ownership check and existing delivery state (§11.3 / §13).
    #[tokio::test]
    async fn epoch_mark_consumed_outcomes() {
        let (b, _path) = fresh("epoch-consume").await;

        let addr = "epoch:consume";
        let owner = "owner-consume";

        // Claim lease.
        let claim_result = b.claim_epoch_lease(addr, owner, 15).await.unwrap();
        let epoch = match claim_result {
            EpochClaimResult::Claimed(ref e) => e.lease_epoch,
            other => panic!("expected Claimed, got {:?}", other),
        };

        // Insert a message addressed to addr.
        let msg = b
            .insert_message(&NewMessage {
                to_addr: addr.to_string(),
                body: "hello".to_string(),
                attention: Attention::Background,
                ..Default::default()
            })
            .await
            .unwrap();
        let mid = msg.id;

        // Not owner (wrong owner_instance_id) → NotOwner wins over all.
        assert_eq!(
            b.mark_consumed_if_current_owner(addr, "impostor", epoch, mid)
                .await
                .unwrap(),
            DeliveryOutcome::NotOwner
        );

        // AckNoOp case: message exists, delivery row pending (fan-out created by insert_message)
        // but we haven't consumed it yet from the correct owner side — wait, the row DOES exist
        // (fan-out created NULL). So this should be pending → Marked on first call.
        // First call with correct owner/epoch → Marked.
        assert_eq!(
            b.mark_consumed_if_current_owner(addr, owner, epoch, mid)
                .await
                .unwrap(),
            DeliveryOutcome::Marked,
            "first mark should return Marked"
        );

        // Second call → AlreadyConsumed.
        assert_eq!(
            b.mark_consumed_if_current_owner(addr, owner, epoch, mid)
                .await
                .unwrap(),
            DeliveryOutcome::AlreadyConsumed,
            "second mark should return AlreadyConsumed"
        );

        // AckNoOp: message with no delivery row at all.
        let msg2 = b
            .insert_message(&NewMessage {
                to_addr: "epoch:other".to_string(),
                body: "other".to_string(),
                attention: Attention::Background,
                ..Default::default()
            })
            .await
            .unwrap();
        // msg2.id was sent to a different address, so no delivery row for addr.
        assert_eq!(
            b.mark_consumed_if_current_owner(addr, owner, epoch, msg2.id)
                .await
                .unwrap(),
            DeliveryOutcome::AckNoOp,
            "no delivery row should return AckNoOp"
        );

        // NotOwner still takes precedence even if delivery row exists.
        assert_eq!(
            b.mark_consumed_if_current_owner(addr, "impostor", epoch, mid)
                .await
                .unwrap(),
            DeliveryOutcome::NotOwner,
            "NotOwner precedence over AlreadyConsumed"
        );
    }

    /// Fan-out creates independent per-recipient delivery rows: acking the primary `to`
    /// recipient does not consume the same message for a cc recipient.
    #[tokio::test]
    async fn epoch_fanout_is_per_recipient() {
        let (b, _path) = fresh("epoch-fanout").await;

        let to = "epoch:fanout:to";
        let cc = "epoch:fanout:cc";
        let owner_to = "owner-to";
        let owner_cc = "owner-cc";

        let to_epoch = match b.claim_epoch_lease(to, owner_to, 15).await.unwrap() {
            EpochClaimResult::Claimed(e) => e.lease_epoch,
            other => panic!("expected to claim, got {:?}", other),
        };
        let cc_epoch = match b.claim_epoch_lease(cc, owner_cc, 15).await.unwrap() {
            EpochClaimResult::Claimed(e) => e.lease_epoch,
            other => panic!("expected cc claim, got {:?}", other),
        };

        let msg = b
            .insert_message(&NewMessage {
                to_addr: to.to_string(),
                cc: Some(cc.to_string()),
                body: "fanout".to_string(),
                attention: Attention::Background,
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(
            b.fetch_undelivered(to).await.unwrap().len(),
            1,
            "to recipient has a pending delivery"
        );
        assert_eq!(
            b.fetch_undelivered(cc).await.unwrap().len(),
            1,
            "cc recipient has an independent pending delivery"
        );

        assert_eq!(
            b.mark_consumed_if_current_owner(to, owner_to, to_epoch, msg.id)
                .await
                .unwrap(),
            DeliveryOutcome::Marked
        );

        assert!(
            b.fetch_undelivered(to).await.unwrap().is_empty(),
            "to recipient consumed"
        );
        assert_eq!(
            b.fetch_undelivered(cc).await.unwrap().len(),
            1,
            "cc recipient is not cross-consumed by to ack"
        );

        assert_eq!(
            b.mark_consumed_if_current_owner(cc, owner_cc, cc_epoch, msg.id)
                .await
                .unwrap(),
            DeliveryOutcome::Marked,
            "cc recipient can still mark its own delivery"
        );
    }

    /// The durable clock must be monotonically non-decreasing across backend reopens.
    #[tokio::test]
    async fn epoch_durable_clock_monotonic() {
        let path = tmp_path("epoch-clock");
        let t2;
        {
            let b = SqliteBackend::open(&path).expect("open1");
            b.init_schema().await.expect("init");
            let t1 = b.durable_clock_now_ms().await.unwrap();
            t2 = b.durable_clock_now_ms().await.unwrap();
            assert!(t2 >= t1, "clock must not go backward within same session");
        }
        // Reopen: clock must be >= last persisted HWM.
        {
            let b2 = SqliteBackend::open(&path).expect("open2");
            b2.init_schema().await.expect("init2");
            let t3 = b2.durable_clock_now_ms().await.unwrap();
            assert!(t3 >= t2, "clock after reopen must not move backward");
        }
    }

    /// Opening the same SQLite store via `open_locked` twice must fail on the second call
    /// while the first lock holder is alive.
    #[tokio::test]
    async fn epoch_store_lock_contention() {
        let path = tmp_path("epoch-lock");
        // Create the DB first so `open_locked` can stat the file for its ID.
        {
            let b = SqliteBackend::open(&path).expect("create db");
            b.init_schema().await.expect("init");
        }

        let _b1 = SqliteBackend::open_locked(&path).expect("first lock must succeed");
        let second = SqliteBackend::open_locked(&path);
        assert!(
            second.is_err(),
            "second open_locked must fail while the first store lock is held"
        );
    }
}
