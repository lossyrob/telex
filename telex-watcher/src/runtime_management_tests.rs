use super::*;
use serde_json::json;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("watcher-management-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        Self {
            root: std::fs::canonicalize(root).unwrap(),
        }
    }

    fn owner(&self) -> (Runtime, LocalManagement) {
        let (runtime, management) = Runtime::open(RuntimeConfig {
            registry: self.root.join("registry.sqlite"),
            daemon: ApplicationDaemonBootstrap::ExactExecutable {
                executable: std::env::current_exe().unwrap(),
            },
            concurrency: 1,
            retention_warning_rows: 10000,
            retention_warning_bytes: 10_000_000,
        })
        .unwrap();
        lock(&runtime.registry)
            .unwrap()
            .start_runtime(&runtime.id, now_ms(), |_| true)
            .unwrap();
        (runtime, management)
    }

    async fn register(&self, runtime: &mut Runtime) {
        let result = runtime.manage(Command::Register {
            json:serde_json::to_vec(&json!({
                "schemaVersion":2,"id":"watch","command":["not-executed"],
                "backendProfile":"fixture","sender":"service:required","target":"agent:recipient",
                "intervalSeconds":1,"timeoutSeconds":1
            })).unwrap(),
            working_directory:self.root.clone(),
        }).await.unwrap();
        assert!(matches!(result, Response::Mutation(result) if result.refusal.is_none()));
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).expect("remove isolated management fixture");
    }
}

async fn health(runtime: &mut Runtime) -> RuntimeHealth {
    match runtime.manage(Command::Health).await.unwrap() {
        Response::Health(health) => health,
        other => panic!("expected runtime health, got {other:?}"),
    }
}

#[tokio::test]
async fn startup_health_waits_for_the_first_membership_reconciliation() {
    let fixture = Fixture::new();
    let (mut runtime, _management) = fixture.owner();
    let initial = health(&mut runtime).await;
    assert!(!initial.production_ready);
    assert_eq!(initial.reconciled_registry_revision, None);
    assert!(initial
        .diagnostics
        .contains(&RuntimeDiagnostic::MembershipReconciliationPending));
    runtime.reconciled_registry_revision = Some(0);
    runtime.clients.reconciliation_needed = false;
    assert!(health(&mut runtime).await.production_ready);
}

#[tokio::test]
async fn registration_before_the_next_tick_invalidates_previously_ready_health() {
    let fixture = Fixture::new();
    let (mut runtime, _management) = fixture.owner();
    runtime.reconciled_registry_revision = Some(0);
    runtime.clients.reconciliation_needed = false;
    assert!(health(&mut runtime).await.production_ready);
    fixture.register(&mut runtime).await;
    let registered = health(&mut runtime).await;
    assert!(!registered.production_ready);
    assert_eq!(registered.registry_revision, 1);
    assert_eq!(registered.reconciled_registry_revision, Some(0));
    assert!(registered
        .diagnostics
        .contains(&RuntimeDiagnostic::MembershipReconciliationPending));
    assert!(registered
        .diagnostics
        .contains(&RuntimeDiagnostic::SenderUnready));
    assert_eq!(registered.senders.len(), 1);
    assert_eq!(registered.senders[0].backend_profile, "fixture");
    assert_eq!(registered.senders[0].required_senders, ["service:required"]);
    assert!(!registered.senders[0].ready);
    assert_eq!(registered.senders[0].logical_store_id, None);
}

#[tokio::test]
async fn a_matching_revision_alone_does_not_prove_required_sender_or_store_readiness() {
    let fixture = Fixture::new();
    let (mut runtime, _management) = fixture.owner();
    fixture.register(&mut runtime).await;
    runtime.reconciled_registry_revision = Some(1);
    runtime.clients.reconciliation_needed = false;
    let observed = health(&mut runtime).await;
    assert!(!observed.production_ready);
    assert!(observed
        .diagnostics
        .contains(&RuntimeDiagnostic::SenderUnready));
    assert!(!observed.senders[0].ready);
}

#[tokio::test]
async fn management_resume_recovers_only_after_original_store_verification() {
    let fixture = Fixture::new();
    let (mut runtime, _management) = fixture.owner();
    fixture.register(&mut runtime).await;
    lock(&runtime.registry)
        .unwrap()
        .bind_store("watch", "store-a")
        .unwrap();
    lock(&runtime.registry)
        .unwrap()
        .bind_store("watch", "store-b")
        .unwrap();
    let refused = runtime
        .manage(Command::Resume { id: "watch".into() })
        .await
        .unwrap();
    assert!(matches!(refused, Response::Mutation(result)
        if result.refusal == Some(Refusal::BlockedReasonUncorrected)));
    lock(&runtime.registry)
        .unwrap()
        .bind_store("watch", "store-a")
        .unwrap();
    assert_eq!(
        lock(&runtime.registry)
            .unwrap()
            .get("watch")
            .unwrap()
            .unwrap()
            .lifecycle,
        Lifecycle::Paused
    );
    let resumed = runtime
        .manage(Command::Resume { id: "watch".into() })
        .await
        .unwrap();
    assert!(matches!(resumed, Response::Mutation(result)
        if result.refusal.is_none() && result.watch.as_ref().is_some_and(|watch| watch.lifecycle == Lifecycle::Active)));
}
