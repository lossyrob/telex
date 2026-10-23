//! The same Watcher transactions and public-client assertions on both stores.
//! Internal install helpers are isolated fixture setup, never the assertion seam.
use super::*;
use crate::client::Clients;
use crate::evidence::*;
use crate::registry_v2::{now_ms, Change, Registry};
use serde_json::json;
use std::collections::BTreeMap;
use telex::application_client::*;

fn registration(iso: &isolation::Isolation, id: &str, sender: &str) -> protocol_v2::Registration {
    protocol_v2::parse_registration(
        &serde_json::to_vec(&json!({
            "schemaVersion":2,"id":id,"command":["unused-detector"],
            "intervalSeconds":1,"timeoutSeconds":5,
            "backendProfile":"watcher-test","sender":sender,"target":"agent:watcher-recipient",
            "initialState":{"cursor":1}
        }))
        .unwrap(),
        &iso.root,
    )
    .unwrap()
}

async fn staged(registry: &mut Registry, clients: &Clients, id: &str) -> Operation {
    let client = clients.get("watcher-test").unwrap();
    registry
        .bind_store(id, &client.logical_store_id().0)
        .unwrap();
    let (watch, attempt) = registry
        .begin_attempt(id, "test-runtime", now_ms())
        .unwrap();
    let result = protocol_v2::parse_result(
        br#"{
        "schemaVersion":2,"outcome":"event","nextState":{"cursor":2},
        "event":{"kind":"fixture.changed","subject":"Changed","body":"Same condition"}
    }"#,
    )
    .unwrap();
    let operation = crate::staging::prepare(
        &client,
        &watch,
        &attempt,
        EventProposal {
            event: result.event.unwrap(),
            next_state: result.next_state.unwrap(),
            terminal: false,
        },
        now_ms(),
    )
    .await
    .unwrap();
    registry.stage(&operation, now_ms()).unwrap();
    operation
}

async fn battery(iso: &isolation::Isolation) {
    let registry_path = iso.root.join("watcher.sqlite");
    let mut registry = Registry::open(&registry_path, 10000, 10_000_000).unwrap();
    for id in [
        "accepted",
        "not-recorded",
        "retained",
        "mismatch",
        "inbound",
    ] {
        registry
            .register(registration(iso, id, "service:watcher-suite"), now_ms())
            .unwrap();
    }
    let mut clients = Clients::new(
        registry.responsibility.clone(),
        ApplicationDaemonBootstrap::InstalledCurrent {
            trusted_root: iso.trusted_root(),
        },
    );
    clients.reconcile(&registry.list().unwrap()).await;
    let client = clients.get("watcher-test").unwrap();
    assert!(clients.health["watcher-test"].ready, "{:?}", clients.health);
    let membership = &clients.health["watcher-test"].membership[0];
    assert_eq!(membership.capability, ApplicationCapability::SendOnly);
    assert!(!membership.receive_ready);
    assert!(membership
        .liveness
        .iter()
        .any(|proof| proof.pid == std::process::id() && proof.alive && proof.start_time.is_some()));

    // A durable send followed by a failed local commit remains frozen across
    // registry replacement. The only recovery action is exact receipt lookup.
    let operation = staged(&mut registry, &clients, "accepted").await;
    assert!(registry
        .reserve_recovery(&operation.operation_id, true, false, now_ms())
        .unwrap());
    let accepted = client.send(operation.envelope.clone()).await.unwrap();
    assert_eq!(accepted.axes.durable_acceptance, EvidenceState::Accepted);
    registry.fail_next_accept_commit = true;
    assert!(registry
        .evidence(
            &operation.operation_id,
            SendEvidence::send(Ok(accepted.clone())),
            now_ms()
        )
        .is_err());
    drop(registry);
    let mut registry = Registry::open(&registry_path, 10000, 10_000_000).unwrap();
    registry
        .start_runtime("replacement", now_ms(), |_| true)
        .unwrap();
    let pending = registry
        .operation(&operation.operation_id)
        .unwrap()
        .unwrap();
    assert_eq!(pending.envelope, operation.envelope);
    assert_eq!(pending.recovery, operation.recovery);
    let proof = client.reconcile_operation(&pending.recovery).await.unwrap();
    registry
        .evidence(
            &operation.operation_id,
            SendEvidence::Reconciliation(Ok(proof)),
            now_ms(),
        )
        .unwrap();
    assert_eq!(
        registry.get("accepted").unwrap().unwrap().state,
        json!({"cursor":2})
    );
    assert_eq!(
        registry.events("accepted", 10).unwrap()[0]
            .receipt
            .as_ref()
            .unwrap()
            .message_id,
        accepted.message_id
    );

    // Exact absence proof authorizes only the original staged request.
    let missing = staged(&mut registry, &clients, "not-recorded").await;
    let absence = client.reconcile_operation(&missing.recovery).await.unwrap();
    assert!(matches!(absence, OperationReconciliation::NotRecorded(_)));
    registry
        .evidence(
            &missing.operation_id,
            SendEvidence::Reconciliation(Ok(absence)),
            now_ms(),
        )
        .unwrap();
    assert!(registry
        .reserve_recovery(&missing.operation_id, true, false, now_ms())
        .unwrap());
    let receipt = client.send(missing.envelope.clone()).await.unwrap();
    registry
        .evidence(
            &missing.operation_id,
            SendEvidence::send(Ok(receipt)),
            now_ms(),
        )
        .unwrap();
    assert_eq!(
        registry
            .get("not-recorded")
            .unwrap()
            .unwrap()
            .last_committed_event_sequence,
        1
    );

    // Pruning completed client operations crosses the exact absence boundary.
    let retained = staged(&mut registry, &clients, "retained").await;
    let cleanup = client
        .cleanup(telex::model::RetentionPolicy {
            completed_before_ms: i64::MAX,
            max_delete: 100,
        })
        .await
        .unwrap();
    assert!(cleanup.operations_deleted >= 1);
    let boundary = client
        .reconcile_operation(&retained.recovery)
        .await
        .unwrap();
    assert!(matches!(
        boundary,
        OperationReconciliation::RetentionBoundaryCrossed { .. }
    ));
    registry
        .evidence(
            &retained.operation_id,
            SendEvidence::Reconciliation(Ok(boundary)),
            now_ms(),
        )
        .unwrap();
    assert!(!registry
        .reserve_recovery(&retained.operation_id, true, false, now_ms())
        .unwrap());
    assert_eq!(
        registry.get("retained").unwrap().unwrap().state,
        json!({"cursor":1})
    );

    // Payload mismatch is public typed evidence, not a log or CLI parse.
    let mismatch = staged(&mut registry, &clients, "mismatch").await;
    let receipt = client.send(mismatch.envelope.clone()).await.unwrap();
    let mut wrong = mismatch.recovery.clone();
    wrong.payload_identity.digest = "0".repeat(64);
    let conflict = client.reconcile_operation(&wrong).await.unwrap_err();
    assert!(matches!(
        conflict,
        ApplicationClientError::OperationMismatch { .. }
    ));
    registry
        .evidence(
            &mismatch.operation_id,
            SendEvidence::Reconciliation(Err(conflict)),
            now_ms(),
        )
        .unwrap();
    assert_eq!(
        registry.get("mismatch").unwrap().unwrap().blocked_reason,
        Some(BlockedReason::OperationIdentityConflict)
    );
    registry
        .change("mismatch", Change::Remove, now_ms())
        .unwrap();
    registry
        .evidence(
            &mismatch.operation_id,
            SendEvidence::send(Ok(receipt)),
            now_ms(),
        )
        .unwrap();
    assert_eq!(
        registry.get("mismatch").unwrap().unwrap().lifecycle,
        Lifecycle::Removed
    );

    // A different responsibility/store binding cannot authorize a local commit.
    let mut foreign = retained.recovery.clone();
    foreign.logical_store_id = LogicalStoreId("foreign-logical-store".into());
    let mismatch = client.reconcile_operation(&foreign).await.unwrap_err();
    assert!(matches!(
        mismatch,
        ApplicationClientError::StoreBindingMismatch { .. }
    ));

    let sender = ApplicationClient::connect_with_daemon(
        ApplicationClientConfig {
            responsibility: ApplicationResponsibility("fixture-inbound-sender".into()),
            backend: Some("watcher-test".into()),
            db_override: None,
        },
        ApplicationDaemonBootstrap::InstalledCurrent {
            trusted_root: iso.trusted_root(),
        },
    )
    .await
    .unwrap();
    assert!(
        sender
            .attach(&[AddressSpec {
                address: "service:inbound-test".into(),
                capability: ApplicationCapability::SendOnly,
                description: None,
                scope: None,
                tags: None,
            }])
            .await
            .ready
    );
    let inbound = sender
        .send(SendRequest {
            operation_id: OperationId("inbound-operation".into()),
            sender: "service:inbound-test".into(),
            to: "service:watcher-suite".into(),
            cc: Vec::new(),
            kind: "fixture.inbound".into(),
            attention: "interrupt".into(),
            requires_disposition: true,
            subject: Some("Inbound routing mistake".into()),
            body: "Do not consume this".into(),
            metadata: None,
            retry_budget: 0,
        })
        .await
        .unwrap();
    assert_eq!(inbound.axes.occupied_at_acceptance, Some(false));
    clients.reconcile(&registry.list().unwrap()).await;
    assert!(clients.health["watcher-test"].ready);
    assert!(clients.health["watcher-test"]
        .membership
        .iter()
        .all(|m| m.inbound_actionable == 0 && !m.receive_ready));
    // If a future/interim client exposes actionable inbound work, the same
    // production readiness reducer must fence it, without consuming anything.
    let mut unexpected = clients.health["watcher-test"].membership[0].clone();
    unexpected.inbound_actionable = 1;
    assert!(!crate::client::membership_ready(&client, &unexpected));

    // A partial attach is non-ready and carries exact compensation evidence.
    let mut other_registry =
        Registry::open(&iso.root.join("colliding.sqlite"), 10000, 10_000_000).unwrap();
    other_registry
        .register(registration(iso, "first", "service:first-free"), now_ms())
        .unwrap();
    other_registry
        .register(
            registration(iso, "second", "service:watcher-suite"),
            now_ms(),
        )
        .unwrap();
    let mut colliding = Clients::new(
        other_registry.responsibility.clone(),
        ApplicationDaemonBootstrap::InstalledCurrent {
            trusted_root: iso.trusted_root(),
        },
    );
    colliding.reconcile(&other_registry.list().unwrap()).await;
    let health = &colliding.health["watcher-test"];
    assert!(!health.ready);
    assert!(health
        .lifecycle
        .as_ref()
        .unwrap()
        .results
        .values()
        .any(|result| matches!(
            result,
            AddressLifecycleResult::Failed(ApplicationClientError::Collision(_))
        )));
    assert!(!health.compensation.is_empty());
    assert!(health.compensation.iter().all(|outcome| outcome.ready));
    colliding.shutdown().await;
    sender.detach("service:inbound-test").await.unwrap();
    assert!(clients.shutdown().await.iter().all(|outcome| outcome.ready));

    actor_lifecycle(iso).await;
}

async fn actor_lifecycle(iso: &isolation::Isolation) {
    use crate::management::{Command, Response};
    let config = RuntimeConfig::installed(iso.root.join("actor.sqlite"), iso.trusted_root());
    let (runtime, management) = Runtime::open(config).unwrap();
    let owner = tokio::spawn(runtime.run());
    let detector = isolation::branch_binary()
        .with_file_name(format!("fake_detector{}", std::env::consts::EXE_SUFFIX));
    assert!(
        detector.is_absolute() && detector.is_file(),
        "build Telex, Watcher fixture, and guardian binaries before the library-only matrix: cargo build -p telex -p telex-watcher --all-features --bins"
    );
    let input = iso.root.join("actor-detector.json");
    std::fs::write(
        &input,
        serde_json::to_vec(&json!({"stdout":serde_json::to_string(&json!({
        "schemaVersion":2,"outcome":"terminal","nextState":null,
        "event":{"kind":"fixture.terminal","subject":"Finished","body":"Done"}
    })).unwrap()}))
        .unwrap(),
    )
    .unwrap();
    let registration = json!({
        "schemaVersion":2,"id":"actor-terminal","command":[detector,input],
        "backendProfile":"watcher-test","sender":"service:actor-suite","target":"agent:watcher-recipient",
        "intervalSeconds":1,"timeoutSeconds":5,"initialState":{"cursor":1},
        "environmentAllowlist":["TELEX_HOME","TELEX_DB","TELEX_INSTALL_ROOT"]
    });
    let response = management
        .execute(Command::Register {
            json: serde_json::to_vec(&registration).unwrap(),
            working_directory: iso.root.clone(),
        })
        .await
        .unwrap();
    assert!(matches!(response,Response::Mutation(ref result) if result.refusal.is_none()));
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let response = management
            .execute(Command::Show {
                id: "actor-terminal".into(),
            })
            .await
            .unwrap();
        if let Response::Watch(Some(watch)) = response {
            if watch.lifecycle == Lifecycle::Terminal {
                assert_eq!(watch.state, serde_json::Value::Null);
                assert_eq!(watch.last_committed_event_sequence, 1);
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "actor failed to complete: {watch:?}"
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let slow = iso.root.join("slow-detector.json");
    std::fs::write(
        &slow,
        r#"{"sleepMs":8000,"stdout":"{\"schemaVersion\":2,\"outcome\":\"idle\"}"}"#,
    )
    .unwrap();
    let mut registration = registration;
    registration["id"] = json!("actor-timeout");
    registration["command"][1] = json!(slow);
    registration["timeoutSeconds"] = json!(3);
    management
        .execute(Command::Register {
            json: serde_json::to_vec(&registration).unwrap(),
            working_directory: iso.root.clone(),
        })
        .await
        .unwrap();
    let first = match management.execute(Command::Health).await.unwrap() {
        Response::Health(health) => health,
        other => panic!("expected health, got {other:?}"),
    };
    tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
    let second = match management.execute(Command::Health).await.unwrap() {
        Response::Health(health) => health,
        other => panic!("expected health, got {other:?}"),
    };
    assert!(
        second.heartbeat_at > first.heartbeat_at,
        "heartbeat must not wait for detector exit"
    );
    management.execute(Command::Shutdown).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(20), owner)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[cfg(feature = "sqlite")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_watcher_runtime_matrix() {
    let _lock = isolation::ENV_LOCK.lock().await;
    let iso = isolation::Isolation::new("watcher-v2-sqlite");
    let _restore = iso.apply_env();
    let profile = telex::profiles::implicit_sqlite(Some(iso.db_path.to_str().unwrap()));
    iso.write_config(&telex::profiles::ConfigFile {
        default: None,
        backends: BTreeMap::from([("watcher-test".into(), profile)]),
    });
    battery(&iso).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn postgres_watcher_runtime_matrix() {
    let _lock = isolation::ENV_LOCK.lock().await;
    let Some(url) = isolation::postgres_url_or_fail_closed("watcher-v2") else {
        return;
    };
    let iso = isolation::Isolation::new("watcher-v2-postgres");
    let _restore = iso.apply_env();
    let schema = format!("watcher_{}", uuid::Uuid::new_v4().simple());
    let mut profile = telex::profiles::implicit_sqlite(None);
    profile.kind = "postgres".into();
    profile.path = None;
    profile.url = Some(url.clone());
    profile.schema = Some(schema.clone());
    profile.auth = Some("password".into());
    if std::env::var("TELEX_PG_PASSWORD").is_ok() {
        profile.password_env = Some("TELEX_PG_PASSWORD".into());
    }
    iso.write_config(&telex::profiles::ConfigFile {
        default: None,
        backends: BTreeMap::from([("watcher-test".into(), profile)]),
    });
    battery(&iso).await;
    iso.stop_daemon();
    let mut config: tokio_postgres::Config = url.parse().expect("valid fixture URL");
    if let Ok(password) = std::env::var("TELEX_PG_PASSWORD") {
        config.password(password);
    }
    let (client, connection) = config
        .connect(telex::backend::postgres::make_tls().unwrap())
        .await
        .unwrap();
    let connection = tokio::spawn(connection);
    client
        .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .unwrap();
    drop(client);
    connection.await.unwrap().unwrap();
}
