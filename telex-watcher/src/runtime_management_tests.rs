use super::*;
use serde_json::json;

/// Hang guard, not a latency assertion. Every management command commits a
/// `synchronous=FULL` SQLite transaction, so the elapsed time follows disk
/// flush latency. On hosted Windows runners the five commands plus Shutdown
/// measured p50 0.5 s, p99 3.1 s and max 7.4 s over 3,162 runs, and opening the
/// fresh registry alone reached 14.7 s. CI exceeded the former 10 s bound (#203).
const DURABLE_MANAGEMENT_HANG_GUARD: Duration = Duration::from_secs(60);

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

    fn open(&self) -> (Runtime, LocalManagement) {
        Runtime::open(RuntimeConfig {
            registry: self.root.join("registry.sqlite"),
            daemon: ApplicationDaemonBootstrap::ExactExecutable {
                executable: std::env::current_exe().unwrap(),
            },
            concurrency: 1,
            retention_warning_rows: 10000,
            retention_warning_bytes: 10_000_000,
        })
        .unwrap()
    }

    fn owner(&self) -> (Runtime, LocalManagement) {
        let (runtime, management) = self.open();
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
        let removed = std::fs::remove_dir_all(&self.root);
        // A second panic while unwinding aborts the whole test harness
        // (0xc0000409 on Windows) and discards the original failure.
        if !std::thread::panicking() {
            removed.expect("remove isolated management fixture");
        }
    }
}

async fn health(runtime: &mut Runtime) -> RuntimeHealth {
    match runtime.manage(Command::Health).await.unwrap() {
        Response::Health(health) => health,
        other => panic!("expected runtime health, got {other:?}"),
    }
}

#[test]
fn fixture_cleanup_failure_while_unwinding_preserves_the_original_panic() {
    let unwound = std::panic::catch_unwind(|| {
        let fixture = Fixture::new();
        std::fs::remove_dir_all(&fixture.root).unwrap();
        panic!("original test failure");
    })
    .unwrap_err();
    assert_eq!(
        unwound.downcast_ref::<&str>(),
        Some(&"original test failure")
    );
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

#[tokio::test]
async fn public_management_removes_opaque_legacy_state_without_v2_admission() {
    // Issue #209 pilot (non-merging): records phases only when PILOT209_OUT is set.
    let pilot_out = crate::pilot209::output_dir();
    let mut pilot_writes = Vec::new();
    for (pilot_state, state) in [
        json!({"checksum": u64::MAX}),
        json!({"nested": [i64::MIN]}),
        json!({"opaque": "\u{ffff}"}),
    ]
    .into_iter()
    .enumerate()
    {
        if pilot_out.is_some() {
            crate::pilot209::begin();
        }
        let pilot = std::time::Instant::now();
        let fixture = Fixture::new();
        crate::pilot209::record("test:fixture-new", pilot);
        let pilot = std::time::Instant::now();
        let mut connection =
            rusqlite::Connection::open(fixture.root.join("registry.sqlite")).unwrap();
        crate::pilot209::attach(&mut connection);
        connection
            .execute_batch("CREATE TABLE watches(id TEXT PRIMARY KEY,state_json TEXT NOT NULL);")
            .unwrap();
        connection
            .execute(
                "INSERT INTO watches VALUES('old-watch',?1)",
                [serde_json::to_string(&state).unwrap()],
            )
            .unwrap();
        drop(connection);
        crate::pilot209::record("test:legacy-seed", pilot);
        let pilot = std::time::Instant::now();
        let (runtime, management) = fixture.open();
        crate::pilot209::record("test:open-composite", pilot);
        let owner = tokio::spawn(runtime.run());
        let pilot_guarded = std::time::Instant::now();
        let pilot_commands =
            tokio::time::timeout(DURABLE_MANAGEMENT_HANG_GUARD, async {
                let registration = |id, initial_state| Command::Register {
                    json: serde_json::to_vec(&json!({
                        "schemaVersion": 2, "id": id, "command": ["not-executed"],
                        "backendProfile": "fixture", "sender": "service:watcher",
                        "target": "agent:recipient", "intervalSeconds": 1, "timeoutSeconds": 1,
                        "initialState": initial_state
                    }))
                    .unwrap(),
                    working_directory: fixture.root.clone(),
                };
                let pilot = std::time::Instant::now();
                let rejected = management
                    .execute(registration("new-watch", state.clone()))
                    .await;
                crate::pilot209::record("test:cmd:register-rejected", pilot);
                let pilot = std::time::Instant::now();
                let removed = management
                    .execute(Command::Remove {
                        id: "old-watch".into(),
                    })
                    .await;
                crate::pilot209::record("test:cmd:remove", pilot);
                let pilot = std::time::Instant::now();
                let shown = management
                    .execute(Command::Show {
                        id: "old-watch".into(),
                    })
                    .await;
                crate::pilot209::record("test:cmd:show", pilot);
                let pilot = std::time::Instant::now();
                let reused = management
                    .execute(registration("old-watch", serde_json::Value::Null))
                    .await;
                crate::pilot209::record("test:cmd:register-reused", pilot);
                let pilot = std::time::Instant::now();
                let attempts = management
                    .execute(Command::Attempts {
                        id: "old-watch".into(),
                        limit: 10,
                    })
                    .await;
                crate::pilot209::record("test:cmd:attempts", pilot);
                let pilot = std::time::Instant::now();
                management.execute(Command::Shutdown).await.unwrap();
                crate::pilot209::record("test:cmd:shutdown", pilot);
                (rejected, removed, shown, reused, attempts)
            })
            .await;
        crate::pilot209::record("test:guarded-commands", pilot_guarded);
        let pilot = std::time::Instant::now();
        let pilot_owner = if pilot_commands.is_ok() {
            Some(tokio::time::timeout(DURABLE_MANAGEMENT_HANG_GUARD, owner).await)
        } else {
            None
        };
        crate::pilot209::record("test:owner-join", pilot);
        if let Some(dir) = &pilot_out {
            let line = json!({
                "schema": "pilot209-opaque/v1",
                "pid": std::process::id(),
                "thread": format!("{:?}", std::thread::current().id()),
                "iteration": std::env::var("PILOT209_ITERATION").ok(),
                "arm": std::env::var("PILOT209_ARM").ok(),
                "state_index": pilot_state,
                "guard_s": DURABLE_MANAGEMENT_HANG_GUARD.as_secs(),
                "commands_ok": pilot_commands.is_ok(),
                "owner_ok": pilot_owner.as_ref().map(|owner| owner.is_ok()),
                "events": crate::pilot209::end(),
            });
            pilot_writes.push((|| -> std::io::Result<()> {
                use std::io::Write;
                std::fs::create_dir_all(dir)?;
                let mut out = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(dir.join(format!("opaque-{}.jsonl", std::process::id())))?;
                out.write_all(format!("{line}\n").as_bytes())
            })());
        }
        let (rejected, removed, shown, reused, attempts) = pilot_commands.unwrap();
        pilot_owner
            .expect("owner joined after commands")
            .unwrap()
            .unwrap()
            .unwrap();

        assert!(matches!(rejected.unwrap(), Response::Mutation(result)
            if result.refusal == Some(Refusal::InvalidRegistration)
                && result.diagnostic.as_deref().is_some_and(|d| d.contains("canonicalization-failed"))));
        assert!(matches!(removed.unwrap(), Response::Mutation(result)
            if result.refusal.is_none()
                && result.watch.as_ref().is_some_and(|w| w.lifecycle == Lifecycle::Removed && w.state == state)));
        assert!(matches!(shown.unwrap(), Response::Watch(Some(watch))
            if watch.schema_version == 1 && watch.lifecycle == Lifecycle::Removed && watch.state == state));
        assert!(matches!(reused.unwrap(), Response::Mutation(result)
            if result.refusal == Some(Refusal::IdAlreadyAllocated)));
        assert!(matches!(attempts.unwrap(), Response::Attempts(attempts) if attempts.is_empty()));
    }
    for write in pilot_writes {
        write.expect("write pilot209 phase record");
    }
}
