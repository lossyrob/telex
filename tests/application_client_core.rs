#![cfg(feature = "sqlite")]

use telex::backend::sqlite::SqliteBackend;
use telex::backend::Backend;
use telex::model::Attention;
use telex::model::{
    now_ms, ApplicationMessageOperation, ApplicationOperationBegin, ApplicationRecordScope,
    CompoundDispositionStep, DeliveryOutcome, EpochClaimResult, NewApplicationOperation,
    NewCompoundStepRecord, NewMessage, RetentionPolicy, StoreDeltaRetentionPolicy,
};

fn db_path(name: &str) -> String {
    std::env::temp_dir()
        .join(format!(
            "telex-application-client-{name}-{}-{}.db",
            std::process::id(),
            now_ms()
        ))
        .to_string_lossy()
        .into_owned()
}

#[tokio::test]
async fn operation_identity_replays_and_rejects_changed_payload() {
    let path = db_path("operations");
    let backend = SqliteBackend::open(&path).unwrap();
    backend.init_schema().await.unwrap();
    let operation = NewApplicationOperation {
        logical_store_id: "store-v1-test".into(),
        application_responsibility: "watcher".into(),
        operation_id: "op-1".into(),
        operation_kind: "send".into(),
        sender: "watcher:sender".into(),
        recipients_json: r#"["target"]"#.into(),
        payload_fingerprint: "fingerprint-a".into(),
        retry_budget: 1,
        created_at_ms: now_ms(),
    };

    assert!(matches!(
        backend
            .begin_application_operation(&operation)
            .await
            .unwrap(),
        ApplicationOperationBegin::Started(_)
    ));
    let authored = backend
        .insert_application_message(
            &NewMessage {
                from_addr: Some("watcher:sender".into()),
                to_addr: "target".into(),
                kind: "note".into(),
                attention: Attention::Background,
                body: "payload".into(),
                sent_at_ms: now_ms(),
                ..Default::default()
            },
            &ApplicationMessageOperation {
                logical_store_id: "store-v1-test".into(),
                application_responsibility: "watcher".into(),
                operation_id: "op-1".into(),
                payload_fingerprint: "fingerprint-a".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        backend
            .application_operation_message("store-v1-test", "watcher", "op-1")
            .await
            .unwrap()
            .unwrap()
            .id,
        authored.id
    );
    assert!(matches!(
        backend
            .begin_application_operation(&operation)
            .await
            .unwrap(),
        ApplicationOperationBegin::Replay(existing)
            if existing.payload_fingerprint == operation.payload_fingerprint
    ));

    let mut changed = operation.clone();
    changed.payload_fingerprint = "fingerprint-b".into();
    assert!(matches!(
        backend.begin_application_operation(&changed).await.unwrap(),
        ApplicationOperationBegin::FingerprintMismatch(existing)
            if existing.payload_fingerprint == operation.payload_fingerprint
    ));
}

#[tokio::test]
async fn exact_delivery_ack_is_bound_to_the_delivery_row() {
    let path = db_path("exact-ack");
    let backend = SqliteBackend::open(&path).unwrap();
    backend.init_schema().await.unwrap();
    backend
        .ensure_address("recipient", None, None, None)
        .await
        .unwrap();
    let claim = backend
        .claim_epoch_lease("recipient", "owner-1", 15)
        .await
        .unwrap();
    let (owner, epoch) = match claim {
        EpochClaimResult::Claimed(claimed) => (claimed.owner_instance_id, claimed.lease_epoch),
        EpochClaimResult::AlreadyOwned { .. } => panic!("fresh address was already owned"),
    };
    let message = backend
        .insert_message(&NewMessage {
            to_addr: "recipient".into(),
            kind: "note".into(),
            attention: Attention::Background,
            body: "body".into(),
            sent_at_ms: now_ms(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        backend
            .mark_delivery_consumed_if_current_owner("recipient", &owner, epoch, 999_999, 1)
            .await
            .unwrap(),
        DeliveryOutcome::NoDelivery
    );
    let delivery = backend
        .delivery_for_recipient(message.id, "recipient")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        backend
            .mark_delivery_consumed_if_current_owner(
                "recipient",
                &owner,
                epoch,
                message.id,
                delivery.id + 1,
            )
            .await
            .unwrap(),
        DeliveryOutcome::DeliveryMismatch
    );
    assert_eq!(
        backend
            .mark_delivery_consumed_if_current_owner(
                "recipient",
                &owner,
                epoch,
                message.id,
                delivery.id,
            )
            .await
            .unwrap(),
        DeliveryOutcome::Marked
    );
    assert!(backend
        .state_deltas(0, 100)
        .await
        .unwrap()
        .iter()
        .any(|delta| delta.axis == "acknowledgment"));
}

#[tokio::test]
async fn delta_cleanup_is_explicit_and_bounded() {
    let path = db_path("delta-cleanup");
    let backend = SqliteBackend::open(&path).unwrap();
    backend.init_schema().await.unwrap();
    for index in 0..10 {
        backend
            .append_state_delta("test", &format!("entity:{index}"), "{}")
            .await
            .unwrap();
    }

    let report = backend
        .cleanup_state_deltas(StoreDeltaRetentionPolicy {
            before_version: 11,
            max_delete: 4,
        })
        .await
        .unwrap();
    assert_eq!(report.deltas_deleted, 4);
    assert_eq!(backend.state_deltas(0, 100).await.unwrap().len(), 6);
}

#[tokio::test]
async fn sqlite_v2_to_v3_migration_preserves_messages() {
    let path = db_path("v2-migration");
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE telex_schema_version (
                     singleton INTEGER NOT NULL DEFAULT 1 UNIQUE,
                     version INTEGER NOT NULL
                 );
                 INSERT INTO telex_schema_version(singleton, version) VALUES (1, 2);
                 CREATE TABLE messages (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     thread_id INTEGER,
                     parent_id INTEGER,
                     from_addr TEXT,
                     to_addr TEXT NOT NULL,
                     cc TEXT,
                     kind TEXT NOT NULL DEFAULT 'note',
                     attention TEXT NOT NULL DEFAULT 'background',
                     requires_disposition INTEGER NOT NULL DEFAULT 0,
                     subject TEXT,
                     body TEXT NOT NULL,
                     metadata TEXT,
                     sent_at_ms INTEGER NOT NULL,
                     created_at_ms INTEGER NOT NULL
                 );
                 INSERT INTO messages(
                     id, thread_id, to_addr, kind, attention, requires_disposition,
                     body, sent_at_ms, created_at_ms
                 ) VALUES (1, 1, 'recipient', 'note', 'background', 0, 'preserved', 1, 1);",
            )
            .unwrap();
    }
    let backend = SqliteBackend::open(&path).unwrap();
    backend.init_schema().await.unwrap();
    assert_eq!(
        backend.get_message(1).await.unwrap().unwrap().body,
        "preserved"
    );
    assert!(backend
        .logical_store_id()
        .await
        .unwrap()
        .starts_with("store-v1-"));
}

#[tokio::test]
async fn compound_steps_and_cleanup_preserve_inflight_work() {
    let path = db_path("compound");
    let backend = SqliteBackend::open(&path).unwrap();
    backend.init_schema().await.unwrap();
    let now = now_ms();
    let steps = vec![
        NewCompoundStepRecord {
            logical_store_id: "store-v1-test".into(),
            application_responsibility: "station".into(),
            operation_id: "compound-1".into(),
            step_id: "reply".into(),
            position: 1,
            step_kind: "reply".into(),
            prerequisites_json: "[]".into(),
            declaration_json: "{}".into(),
            created_at_ms: now,
        },
        NewCompoundStepRecord {
            logical_store_id: "store-v1-test".into(),
            application_responsibility: "station".into(),
            operation_id: "compound-1".into(),
            step_id: "close".into(),
            position: 2,
            step_kind: "disposition".into(),
            prerequisites_json: r#"["reply"]"#.into(),
            declaration_json: "{}".into(),
            created_at_ms: now,
        },
    ];
    backend.declare_compound_steps(&steps).await.unwrap();
    assert!(backend
        .complete_compound_step(
            "store-v1-test",
            "station",
            "compound-1",
            "reply",
            "done",
            Some("{}"),
            None,
        )
        .await
        .is_err());
    backend
        .complete_compound_step(
            "store-v1-test",
            "station",
            "compound-1",
            "reply",
            "accepted",
            Some("{}"),
            None,
        )
        .await
        .unwrap();

    let report = backend
        .cleanup_application_records(
            &ApplicationRecordScope {
                logical_store_id: "store-v1-test".into(),
                application_responsibility: "station".into(),
            },
            RetentionPolicy {
                completed_before_ms: i64::MAX,
                max_delete: 100,
            },
        )
        .await
        .unwrap();
    assert_eq!(report.compound_steps_deleted, 0);
    let remaining = backend
        .compound_steps("store-v1-test", "station", "compound-1")
        .await
        .unwrap();
    assert_eq!(remaining.len(), 2);
    assert_eq!(remaining[1].step_id, "close");
    assert_eq!(remaining[1].state, "pending");
}

#[tokio::test]
async fn compound_terminal_disposition_is_fenced_and_atomic() {
    let path = db_path("compound-disposition");
    let backend = SqliteBackend::open(&path).unwrap();
    backend.init_schema().await.unwrap();
    backend
        .ensure_address("recipient", None, None, None)
        .await
        .unwrap();
    let claim = backend
        .claim_epoch_lease("recipient", "owner-1", 15)
        .await
        .unwrap();
    let (owner, epoch) = match claim {
        EpochClaimResult::Claimed(claimed) => (claimed.owner_instance_id, claimed.lease_epoch),
        EpochClaimResult::AlreadyOwned { .. } => panic!("fresh address was already owned"),
    };
    let message = backend
        .insert_message(&NewMessage {
            to_addr: "recipient".into(),
            kind: "note".into(),
            attention: Attention::Background,
            requires_disposition: true,
            body: "body".into(),
            sent_at_ms: now_ms(),
            ..Default::default()
        })
        .await
        .unwrap();
    let delivery = backend
        .delivery_for_recipient(message.id, "recipient")
        .await
        .unwrap()
        .unwrap();
    let now = now_ms();
    backend
        .declare_compound_steps(&[
            NewCompoundStepRecord {
                logical_store_id: "store-v1-test".into(),
                application_responsibility: "station".into(),
                operation_id: "compound-disposition".into(),
                step_id: "reply".into(),
                position: 1,
                step_kind: "reply".into(),
                prerequisites_json: "[]".into(),
                declaration_json: "{}".into(),
                created_at_ms: now,
            },
            NewCompoundStepRecord {
                logical_store_id: "store-v1-test".into(),
                application_responsibility: "station".into(),
                operation_id: "compound-disposition".into(),
                step_id: "close".into(),
                position: 2,
                step_kind: "disposition".into(),
                prerequisites_json: r#"["reply"]"#.into(),
                declaration_json: "{}".into(),
                created_at_ms: now,
            },
        ])
        .await
        .unwrap();
    let effect = CompoundDispositionStep {
        logical_store_id: "store-v1-test".into(),
        application_responsibility: "station".into(),
        operation_id: "compound-disposition".into(),
        step_id: "close".into(),
        outcome_json: Some("{}".into()),
        recovery_json: None,
    };
    let (row, outcome) = backend
        .application_disposition_with_ack(
            "recipient",
            &owner,
            epoch,
            message.id,
            delivery.id,
            "handled",
            None,
            Some("station:sender"),
            None,
            Some(&effect),
        )
        .await
        .unwrap();
    assert!(row.is_none());
    assert_eq!(outcome, DeliveryOutcome::PrerequisiteIncomplete);
    assert!(backend
        .dispositions_for(message.id)
        .await
        .unwrap()
        .is_empty());
    backend
        .complete_compound_step(
            "store-v1-test",
            "station",
            "compound-disposition",
            "reply",
            "accepted",
            Some("{}"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        backend
            .mark_delivery_consumed_if_current_owner(
                "recipient",
                &owner,
                epoch,
                message.id,
                delivery.id,
            )
            .await
            .unwrap(),
        DeliveryOutcome::Marked
    );
    let (disposition, outcome) = backend
        .application_disposition_with_ack(
            "recipient",
            &owner,
            epoch,
            message.id,
            delivery.id,
            "handled",
            None,
            Some("station:sender"),
            None,
            Some(&effect),
        )
        .await
        .unwrap();
    assert!(disposition.is_some());
    assert_eq!(outcome, DeliveryOutcome::AlreadyConsumed);
    assert_eq!(
        backend
            .compound_steps("store-v1-test", "station", "compound-disposition")
            .await
            .unwrap()[1]
            .state,
        "accepted"
    );
    backend.reset_epoch_lease("recipient").await.unwrap();
    let successor = backend
        .claim_epoch_lease("recipient", "owner-2", 15)
        .await
        .unwrap();
    assert!(matches!(successor, EpochClaimResult::Claimed(_)));
    let (stale_row, stale_outcome) = backend
        .application_disposition_with_ack(
            "recipient",
            &owner,
            epoch,
            message.id,
            delivery.id,
            "closed",
            None,
            Some("station:sender"),
            None,
            None,
        )
        .await
        .unwrap();
    assert!(stale_row.is_none());
    assert_eq!(stale_outcome, DeliveryOutcome::NotOwner);
    assert_eq!(backend.dispositions_for(message.id).await.unwrap().len(), 1);
}
