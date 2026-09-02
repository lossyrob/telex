//! Station-intent reconciliation behavior (issue #106 / ADR 0052).
//!
//! Covers the issue's test matrix at the daemon-core level: restoration, idempotence, the
//! tombstone guarantee, CC-watermark preservation, ownership and pull-waiter precedence, the
//! credential security rules, version skew, and the bounded-scan properties. Process-level rows
//! (T1, T7, T8, T16, T21, T26) live in `daemon_process_sqlite.rs` behind the `station_intent_`
//! filter so they also run on macOS in CI.

#![cfg(feature = "sqlite")]

use std::path::PathBuf;
use std::time::Duration;

use telex::daemon::test_support::{register_request, TestDaemon};
use telex::daemon_ipc::{IntentRecoveryState, NeedsAttachReason, Request, Response};
use telex::intent_test_support::{
    live_intent, register_test_handler_kind, register_test_producer_root, write_credential_file,
    FakeProducer, ProducerBehavior,
};
use telex::station_intent::{
    IntentEvidence, ProducerIdentity, StationIntentV1, STATION_INTENT_MAX_COUNT,
    STATION_INTENT_PENDING_TTL,
};

/// One fully wired scenario: a daemon, a store, a fake producer, a registered credential root, and
/// a `Live` intent the daemon can actually verify.
struct Scenario {
    daemon: TestDaemon,
    store_key: String,
    intent: StationIntentV1,
    producer: FakeProducer,
    credential_path: PathBuf,
}

impl Scenario {
    async fn new(label: &str, behavior: ProducerBehavior) -> Self {
        Self::with_session(label, behavior, "sess-1", "addr:station").await
    }

    async fn with_session(
        label: &str,
        behavior: ProducerBehavior,
        session: &str,
        address: &str,
    ) -> Self {
        register_test_handler_kind();
        let daemon = TestDaemon::new(label);
        let store_key = daemon.store_key(label);
        // Reconciliation is open-existing-only: it never brings a store into existence as a side
        // effect of restoring a handler. In the real world the store already exists because the
        // session attached to it, so open it here rather than weakening that rule.
        let _ = daemon.open_backend(&store_key).await;
        let producer_dir = daemon.root().join("producer");
        // Deliberately NOT pre-created: `ensure_owner_private_producer_root` creates a fresh root
        // with the owner-only descriptor, and *validates* (never rewrites) one that already exists.
        // A root created by an ordinary `mkdir` under a broadly-ACLed tree fails closed, which is
        // the intended posture.
        let root_id = format!("test_root_{label}_{}", std::process::id());
        let root = register_test_producer_root(&root_id, &producer_dir);
        let secret = "s".repeat(64);
        let producer = FakeProducer::start(&root, session, &secret, behavior).await;
        let credential_path = root.join(format!("{session}.json"));
        write_credential_file(&credential_path, &secret);
        let intent = live_intent(
            &store_key,
            session,
            address,
            daemon.singleton_hash(),
            &producer,
            &root_id,
            &credential_path,
        );
        daemon
            .intent_store()
            .write_atomic(&intent)
            .expect("seed intent");
        Self {
            daemon,
            store_key,
            intent,
            producer,
            credential_path,
        }
    }

    /// Re-seed the scope with a mutated copy of the fixture intent.
    fn reseed(&self, intent: &StationIntentV1) {
        self.daemon
            .intent_store()
            .write_atomic(intent)
            .expect("reseed intent");
    }

    fn intent_state(&self) -> IntentRecoveryState {
        self.daemon
            .intent_statuses()
            .into_iter()
            .find(|row| row.address == self.intent.address)
            .map(|row| row.state)
            .expect("an intent row for the fixture address")
    }

    fn failure_code(&self) -> Option<String> {
        self.daemon
            .intent_statuses()
            .into_iter()
            .find(|row| row.address == self.intent.address)
            .and_then(|row| row.failure_code)
    }

    async fn status(&self) -> telex::daemon_ipc::DaemonStatus {
        match self
            .daemon
            .request(Request::Status {
                store_key: Some(self.store_key.clone()),
                detail: true,
                proof: Some(self.daemon.admin_cap().to_string()),
            })
            .await
        {
            Response::StatusReport { status } => status,
            other => panic!("expected a status report, got {other:?}"),
        }
    }

    async fn member_push_registered(&self) -> bool {
        self.status()
            .await
            .members
            .iter()
            .any(|m| m.address == self.intent.address && m.push_registered)
    }
}

// ---------------------------------------------------------------------------------------------
// Negative controls: these fail loudly if the feature silently stops working.
// ---------------------------------------------------------------------------------------------

/// Negative control for "the reconciler never runs": if reconciliation stops happening, this test
/// fails rather than a subtler assertion elsewhere quietly passing.
#[tokio::test]
async fn station_intent_reconciler_actually_runs_and_restores_push() {
    let scenario = Scenario::new("intent-restore", ProducerBehavior::Healthy).await;
    assert!(
        !scenario.member_push_registered().await,
        "precondition: no member exists before the first pass"
    );

    let report = scenario.daemon.reconcile_once().await;
    assert_eq!(report.scanned, 1, "the pass must attempt the seeded intent");
    assert_eq!(report.restored, 1, "a verifiable producer must be restored");
    assert!(
        scenario.member_push_registered().await,
        "reconciliation must create a push member"
    );
    assert_eq!(scenario.intent_state(), IntentRecoveryState::Restored);
}

/// Negative control for the single most dangerous regression: reconciliation resurrecting a
/// station the user explicitly detached.
#[tokio::test]
async fn station_intent_never_resurrects_a_tombstoned_station() {
    let scenario = Scenario::new("intent-tombstone", ProducerBehavior::Healthy).await;
    // Detach durably, exactly as `telex detach` does.
    let detach = scenario
        .daemon
        .request(Request::Detach {
            store_key: scenario.store_key.clone(),
            session_id: scenario.intent.session_id.clone(),
            address: scenario.intent.address.clone(),
        })
        .await;
    assert!(matches!(detach, Response::Ack { .. }), "{detach:?}");

    // The detach revoked the local intent; force it back to `Live` to model the hostile case where
    // a stale or hand-edited manifest survives. The durable tombstone must still win.
    let mut resurrected = scenario.intent.clone();
    resurrected.state = IntentRecoveryState::Live;
    resurrected.generation = 99;
    scenario.reseed(&resurrected);

    let report = scenario.daemon.reconcile_once().await;
    assert_eq!(report.restored, 0, "a tombstoned station must never return");
    assert!(!scenario.member_push_registered().await);
    assert_eq!(scenario.intent_state(), IntentRecoveryState::Revoked);

    // And the durable tombstone is still there, byte-identical: the reconcile path must not be able
    // to clear it.
    let backend = scenario.daemon.open_backend(&scenario.store_key).await;
    let tombstone = backend
        .detach_tombstone(&scenario.intent.session_id, &scenario.intent.address)
        .await
        .expect("tombstone query");
    assert!(
        tombstone.is_some(),
        "the reconcile pass must not clear a durable detach tombstone"
    );
}

// ---------------------------------------------------------------------------------------------
// T24 / T23: idempotence and no retry storm
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn station_intent_repeated_passes_are_idempotent_with_no_retry_storm() {
    let scenario = Scenario::new("intent-idempotent", ProducerBehavior::Healthy).await;
    let first = scenario.daemon.reconcile_once().await;
    assert_eq!(first.restored, 1);

    let mut no_ops = 0;
    for _ in 0..3 {
        let report = scenario.daemon.reconcile_once().await;
        assert_eq!(
            report.restored, 0,
            "a restored member must not be recreated"
        );
        assert_eq!(report.failed, 0, "an idempotent pass must not fail");
        no_ops += report.refreshed_no_op;
    }
    assert_eq!(no_ops, 3, "each later pass is a no-op refresh");

    let index = scenario.daemon.intent_index();
    let entry = index
        .entries
        .values()
        .next()
        .expect("an index entry for the fixture");
    assert_eq!(
        entry.consecutive_failures, 0,
        "no-op refreshes must not accumulate failure state"
    );
    assert_eq!(
        entry.next_attempt_ms, None,
        "a healthy intent must not be scheduled into backoff"
    );
}

#[tokio::test]
async fn station_intent_hung_producers_are_bounded_and_the_cursor_resumes() {
    let scenario = Scenario::new("intent-hung-budget", ProducerBehavior::Hang).await;
    let count = telex::daemon_reconcile::RECONCILE_MAX_CONCURRENCY * 4 + 2;
    for index in 1..count {
        let mut intent = scenario.intent.clone();
        intent.address = format!("addr:hung-{index}");
        scenario
            .daemon
            .intent_store()
            .write_atomic(&intent)
            .expect("seed another hung intent");
    }

    let started = std::time::Instant::now();
    let first = scenario.daemon.reconcile_once().await;
    assert!(
        started.elapsed() <= telex::daemon_reconcile::RECONCILE_PASS_DEADLINE,
        "a hung producer set must stay within the pass deadline"
    );
    assert!(
        first.deadline_reached,
        "the deadline must truncate the scope"
    );
    assert!(
        first.scanned < count,
        "the first pass must leave work for the cursor to resume"
    );
    let index = scenario.daemon.intent_index();
    let attempted = index
        .entries
        .values()
        .filter(|entry| entry.last_attempt_ms.is_some())
        .collect::<Vec<_>>();
    assert!(!attempted.is_empty());
    assert!(
        attempted
            .iter()
            .all(|entry| entry.failure_code.as_deref() == Some("probe_timeout")),
        "each attempted hung producer must name the timeout that bounded it"
    );

    let second = scenario.daemon.reconcile_once().await;
    assert!(
        second.scanned > 0,
        "the next pass must resume with intents the first deadline skipped"
    );
}

#[tokio::test]
async fn station_intent_skipped_pass_does_not_consume_wall_clock_gc() {
    let scenario = Scenario::new("intent-gc-cadence", ProducerBehavior::Healthy).await;
    let mut expired = scenario.intent.clone();
    expired.address = "addr:expired-pending".to_string();
    expired.state = IntentRecoveryState::Pending;
    expired.created_at_ms =
        telex::model::now_ms() - STATION_INTENT_PENDING_TTL.as_millis() as i64 - 1;
    expired.updated_at_ms = expired.created_at_ms;
    expired.armed = None;
    expired.evidence = IntentEvidence::default();
    let expired_id = expired.id();
    scenario
        .daemon
        .intent_store()
        .write_atomic(&expired)
        .expect("seed expired pending intent");

    scenario.daemon.set_draining_for_test(true);
    let skipped = scenario.daemon.reconcile_once().await;
    assert!(!skipped.ran);
    assert!(
        scenario.daemon.intent_store().load(&expired_id).is_ok(),
        "a skipped pass must not run GC"
    );

    scenario.daemon.set_draining_for_test(false);
    scenario.daemon.reconcile_once().await;
    assert!(
        scenario.daemon.intent_store().load(&expired_id).is_err(),
        "the next runnable pass must perform the due wall-clock GC"
    );
    assert!(
        scenario
            .daemon
            .intent_store()
            .load(&scenario.intent.id())
            .is_ok(),
        "GC must retain the live intent"
    );
}

#[tokio::test]
async fn station_intent_failure_ladder_reaches_quarantine() {
    let scenario = Scenario::new("intent-quarantine", ProducerBehavior::WrongNonce).await;
    for attempt in 1..=telex::daemon_reconcile::RECONCILE_QUARANTINE_AFTER {
        if attempt > 1 {
            let store = scenario.daemon.intent_store();
            let mut persisted = store.load(&scenario.intent.id()).expect("load evidence");
            persisted.evidence.next_attempt_ms = None;
            assert!(
                store
                    .write_cas(persisted.generation, &persisted)
                    .expect("clear retry delay"),
                "the test owns the manifest generation"
            );
            scenario.daemon.clear_intent_index();
        }
        let report = scenario.daemon.reconcile_once().await;
        assert_eq!(report.failed, 1, "attempt {attempt} must fail");
    }

    let entry = scenario
        .daemon
        .intent_index()
        .entries
        .values()
        .next()
        .expect("quarantined index entry")
        .clone();
    assert_eq!(entry.state, IntentRecoveryState::Quarantined);
    assert_eq!(
        entry.consecutive_failures,
        telex::daemon_reconcile::RECONCILE_QUARANTINE_AFTER
    );
    assert!(
        entry
            .next_attempt_ms
            .is_some_and(|next| next > telex::model::now_ms()),
        "quarantine must park the intent"
    );
}

#[tokio::test]
async fn station_intent_terminal_park_is_not_inherited_as_live_by_a_successor() {
    let scenario = Scenario::new("intent-terminal-successor", ProducerBehavior::Healthy).await;
    let mut legacy = scenario.intent.clone();
    legacy.producer.protocol.max = telex::daemon_reconcile::BRIDGE_PROBE_MIN_PROTOCOL - 1;
    legacy.producer.protocol.min = legacy.producer.protocol.max;
    scenario.reseed(&legacy);

    let first = scenario.daemon.reconcile_once().await;
    assert_eq!(first.inert, 1);
    let persisted = scenario
        .daemon
        .intent_store()
        .load(&legacy.id())
        .expect("reload terminal evidence");
    assert_eq!(
        persisted.evidence.failure_code.as_deref(),
        Some("legacy_producer")
    );
    assert_eq!(
        persisted.evidence.next_attempt_ms, None,
        "process-scoped terminal parks must remain in-memory only"
    );

    scenario.daemon.clear_intent_index();
    let successor = scenario.daemon.reconcile_once().await;
    assert_eq!(
        successor.inert, 1,
        "a successor must classify the terminal intent instead of skipping it as healthy"
    );
    let row = scenario
        .daemon
        .intent_statuses()
        .into_iter()
        .find(|row| row.address == legacy.address)
        .expect("terminal status row");
    assert_eq!(row.state, IntentRecoveryState::LegacyProducer);
    assert!(!row.state.is_recoverable());
}

// ---------------------------------------------------------------------------------------------
// Producer verification: T9, T10, T28
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn station_intent_probe_failures_map_to_their_own_states() {
    // A forged/replayed answer is a *failure* (retryable), not a terminal state.
    let wrong_nonce = Scenario::new("intent-nonce", ProducerBehavior::WrongNonce).await;
    let report = wrong_nonce.daemon.reconcile_once().await;
    assert_eq!(report.failed, 1);
    assert_eq!(
        wrong_nonce.failure_code().as_deref(),
        Some("probe_nonce_mismatch")
    );
    assert!(!wrong_nonce.member_push_registered().await);

    let wrong_session = Scenario::new("intent-session", ProducerBehavior::WrongSession).await;
    let report = wrong_session.daemon.reconcile_once().await;
    assert_eq!(report.failed, 1);
    assert_eq!(
        wrong_session.failure_code().as_deref(),
        Some("probe_session_mismatch")
    );

    // A rotated/failed secret is a failure, and no member is created.
    let bad_secret = Scenario::new("intent-secret", ProducerBehavior::RejectSecret).await;
    let report = bad_secret.daemon.reconcile_once().await;
    assert_eq!(report.failed, 1);
    assert!(!bad_secret.member_push_registered().await);
}

#[tokio::test]
async fn station_intent_legacy_producer_is_legacy_not_failed() {
    let scenario = Scenario::new("intent-legacy", ProducerBehavior::LegacyProtocol).await;
    let report = scenario.daemon.reconcile_once().await;
    assert_eq!(
        report.failed, 0,
        "a pre-probe producer must not be reported as a failure"
    );
    assert_eq!(report.inert, 1);
    assert_eq!(scenario.intent_state(), IntentRecoveryState::LegacyProducer);
    assert!(!scenario.member_push_registered().await);

    // A producer that *declares* an old protocol is classified without even connecting.
    let declared = Scenario::new("intent-legacy-declared", ProducerBehavior::Healthy).await;
    let mut old = declared.intent.clone();
    old.producer.protocol.max = 1;
    old.producer.protocol.min = 1;
    old.generation = 2;
    declared.reseed(&old);
    let report = declared.daemon.reconcile_once().await;
    assert_eq!(report.failed, 0);
    assert_eq!(declared.intent_state(), IntentRecoveryState::LegacyProducer);
}

#[tokio::test]
async fn station_intent_pid_reuse_and_foreign_identity_are_never_verified() {
    // T10: a pid that now belongs to a different process (different start time) must not verify.
    let reused = Scenario::new("intent-pid-reuse", ProducerBehavior::Healthy).await;
    let mut mutated = reused.intent.clone();
    mutated.producer.start_time = mutated.producer.start_time.wrapping_add(1);
    mutated.generation = 2;
    reused.reseed(&mutated);
    let report = reused.daemon.reconcile_once().await;
    assert_eq!(report.restored, 0);
    assert_eq!(
        reused.failure_code().as_deref(),
        Some("producer_identity_mismatch")
    );

    // Cross-host negative: an intent recorded on another machine is never restored here, which is
    // what makes a synced or network-mounted home directory safe.
    let foreign = Scenario::new("intent-foreign-host", ProducerBehavior::Healthy).await;
    let mut other_host = foreign.intent.clone();
    other_host.producer.host_id = "0".repeat(32);
    other_host.generation = 2;
    foreign.reseed(&other_host);
    let report = foreign.daemon.reconcile_once().await;
    assert_eq!(report.restored, 0);
    assert_eq!(
        foreign.failure_code().as_deref(),
        Some("foreign_host_or_boot")
    );

    // Same for a different boot of this machine.
    let other_boot_scenario = Scenario::new("intent-foreign-boot", ProducerBehavior::Healthy).await;
    let mut other_boot = other_boot_scenario.intent.clone();
    other_boot.producer.boot_id = "1".repeat(32);
    other_boot.generation = 2;
    other_boot_scenario.reseed(&other_boot);
    let report = other_boot_scenario.daemon.reconcile_once().await;
    assert_eq!(report.restored, 0);
}

#[tokio::test]
async fn station_intent_credential_rules_fail_closed_without_connecting() {
    // T28: outside the registered root.
    let outside = Scenario::new("intent-cred-outside", ProducerBehavior::Healthy).await;
    let elsewhere = outside.daemon.root().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("elsewhere");
    let stray = elsewhere.join("registry.json");
    write_credential_file(&stray, &"s".repeat(64));
    let mut escaped = outside.intent.clone();
    escaped.producer.credential.path = stray;
    escaped.generation = 2;
    outside.reseed(&escaped);
    outside.daemon.reconcile_once().await;
    assert_eq!(outside.intent_state(), IntentRecoveryState::Insecure);
    assert_eq!(
        outside.failure_code().as_deref(),
        Some("credential_outside_root")
    );

    // An unregistered root id never dereferences a path at all.
    let unregistered = Scenario::new("intent-cred-root", ProducerBehavior::Healthy).await;
    let mut unknown_root = unregistered.intent.clone();
    unknown_root.producer.credential.root_id = "root_that_was_never_registered".to_string();
    unknown_root.generation = 2;
    unregistered.reseed(&unknown_root);
    unregistered.daemon.reconcile_once().await;
    assert_eq!(
        unregistered.failure_code().as_deref(),
        Some("credential_root_unregistered")
    );

    // A credential older than `max_age_ms` is `Unverifiable` with no secret read and no probe.
    let stale = Scenario::new("intent-cred-stale", ProducerBehavior::Healthy).await;
    let mut aged = stale.intent.clone();
    aged.producer.credential.max_age_ms = 1;
    aged.generation = 2;
    stale.reseed(&aged);
    tokio::time::sleep(Duration::from_millis(25)).await;
    stale.daemon.reconcile_once().await;
    assert_eq!(stale.intent_state(), IntentRecoveryState::Unverifiable);
    assert_eq!(stale.failure_code().as_deref(), Some("credential_stale"));

    // A missing credential field is unverifiable rather than a crash or a guess.
    let malformed = Scenario::new("intent-cred-field", ProducerBehavior::Healthy).await;
    std::fs::remove_file(&malformed.credential_path).expect("remove credential");
    telex::platform_fs::write_owner_only_file_atomic(&malformed.credential_path, b"{\"other\":1}")
        .expect("rewrite credential");
    malformed.daemon.reconcile_once().await;
    assert_eq!(
        malformed.failure_code().as_deref(),
        Some("credential_field_missing")
    );
}

#[tokio::test]
async fn station_intent_dead_producer_is_not_restored() {
    // T8/T9 core half: the producer is gone, so nothing is restored and nothing is wedged.
    let scenario = Scenario::new("intent-dead-producer", ProducerBehavior::Healthy).await;
    scenario.producer.kill();
    // Give the listener a moment to actually stop accepting.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let report = scenario.daemon.reconcile_once().await;
    assert_eq!(report.restored, 0);
    assert!(!scenario.member_push_registered().await);
}

// ---------------------------------------------------------------------------------------------
// Anti-downgrade: T25, T18, T19, T6
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn station_intent_matched_version_pull_register_never_downgrades_push() {
    // T25: a pull-only `Register` over a live push intent, on a same-version daemon, must either
    // reconcile to push or return the typed error — never create a pull-only member.
    //
    // Both branches are exercised, because "never a pull-only member" is only meaningful if the
    // failing case is checked too.
    let scenario = Scenario::new("intent-antidowngrade", ProducerBehavior::Healthy).await;
    scenario.daemon.reconcile_once().await;
    assert!(scenario.member_push_registered().await);

    // Model a daemon replacement: the durable intent survives, the in-memory member does not, and
    // no explicit detach ever happened (so there is no tombstone).
    scenario.daemon.forget_member(
        &scenario.store_key,
        &scenario.intent.session_id,
        &scenario.intent.address,
    );
    assert!(!scenario.member_push_registered().await);

    // Branch 1: the producer is still live, so the guard reconciles to push and the incoming
    // registration is treated as a refresh of the now-push member.
    let response = scenario
        .daemon
        .request(register_request(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address,
        ))
        .await;
    assert!(
        matches!(response, Response::Registered { .. }),
        "a recoverable push intent should reconcile rather than error: {response:?}"
    );
    assert!(
        scenario.member_push_registered().await,
        "a registration over a live push intent must never yield a pull-only member"
    );

    // Branch 2: the producer is gone, so push cannot be proven recoverable and the daemon must
    // refuse with a typed reason rather than silently downgrading to pull.
    scenario.producer.kill();
    tokio::time::sleep(Duration::from_millis(50)).await;
    scenario.daemon.forget_member(
        &scenario.store_key,
        &scenario.intent.session_id,
        &scenario.intent.address,
    );
    let refused = scenario
        .daemon
        .request(register_request(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address,
        ))
        .await;
    match refused {
        Response::Error {
            code,
            needs_attach_reason,
            ..
        } => {
            assert_eq!(code, telex::daemon_ipc::ERROR_INCOMPATIBLE);
            assert_eq!(
                needs_attach_reason,
                Some(NeedsAttachReason::PushIntentUnrecoverable),
                "the refusal must be typed so a client can render a recovery path"
            );
        }
        other => panic!("expected a typed anti-downgrade refusal, got {other:?}"),
    }
    assert!(
        !scenario.member_push_registered().await,
        "no member at all is correct here; a pull-only member would be the downgrade"
    );
    let status = scenario.status().await;
    assert!(
        !status
            .members
            .iter()
            .any(|m| m.address == scenario.intent.address),
        "the refused registration must not have created any member"
    );
}

#[tokio::test]
async fn station_intent_live_pull_waiter_still_wins_and_is_deferred_not_failed() {
    // T6 negative control: pull-waiter precedence is preserved, and the intent waits rather than
    // being permanently failed.
    let scenario = Scenario::new("intent-pullwaiter", ProducerBehavior::Healthy).await;
    // Withhold the intent while the pull station is established. Otherwise the (correct)
    // anti-downgrade guard reconciles it to push during `Register`, and there would never be a pull
    // waiter to test precedence against.
    let intent_path = scenario
        .daemon
        .intent_store()
        .path_for(&scenario.intent.id());
    std::fs::remove_file(&intent_path).expect("withhold the intent during setup");

    let register = scenario
        .daemon
        .request(register_request(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address,
        ))
        .await;
    assert!(
        matches!(register, Response::Registered { .. }),
        "{register:?}"
    );

    let daemon_handle = scenario.daemon.handle();
    let waiter = tokio::spawn({
        let store_key = scenario.store_key.clone();
        let session = scenario.intent.session_id.clone();
        let address = scenario.intent.address.clone();
        async move {
            daemon_handle
                .request(Request::Wait {
                    store_key,
                    session_id: session,
                    address,
                    attention: None,
                    min_attention: None,
                    wake_on_cc: false,
                    timeout_ms: Some(2_000),
                    waiter_pid: Some(std::process::id()),
                    waiter_start_time: None,
                })
                .await
        }
    });
    // Let the waiter arm.
    tokio::time::sleep(Duration::from_millis(150)).await;
    // Now the intent exists again — the shape a daemon replacement leaves behind while a pull
    // fallback is already running.
    scenario.reseed(&scenario.intent);

    let report = scenario.daemon.reconcile_once().await;
    assert_eq!(
        report.deferred_pull_waiter, 1,
        "a live armed pull waiter must win, and the intent must defer rather than fail"
    );
    assert_eq!(report.failed, 0);
    assert_eq!(
        scenario.intent_state(),
        IntentRecoveryState::DeferredPullWaiter
    );
    let _ = waiter.await;
}

// ---------------------------------------------------------------------------------------------
// Multi-station / multi-store exactness: T13, T14
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn station_intent_restores_each_binding_exactly() {
    register_test_handler_kind();
    let daemon = TestDaemon::new("intent-multi");
    let store_a = daemon.store_key("multi-a");
    let store_b = daemon.store_key("multi-b");
    let _ = daemon.open_backend(&store_a).await;
    let _ = daemon.open_backend(&store_b).await;
    let producer_dir = daemon.root().join("producer");
    // Deliberately NOT pre-created: `ensure_owner_private_producer_root` creates a fresh root
    // with the owner-only descriptor, and *validates* (never rewrites) one that already exists.
    // A root created by an ordinary `mkdir` under a broadly-ACLed tree fails closed, which is
    // the intended posture.
    let root_id = format!("multi_root_{}", std::process::id());
    let root = register_test_producer_root(&root_id, &producer_dir);
    let secret = "m".repeat(64);
    let producer =
        FakeProducer::start(&root, "sess-multi", &secret, ProducerBehavior::Healthy).await;
    let credential = root.join("sess-multi.json");
    write_credential_file(&credential, &secret);

    let mut seeded = Vec::new();
    for (store, address, watermark) in [
        (&store_a, "addr:one", Some(11)),
        (&store_a, "addr:two", Some(22)),
        (&store_b, "addr:one", Some(33)),
    ] {
        let mut intent = live_intent(
            store,
            "sess-multi",
            address,
            daemon.singleton_hash(),
            &producer,
            &root_id,
            &credential,
        );
        // T3's precondition: the CC watermark must survive reconciliation unchanged, or every CC
        // message committed during a restart gap becomes permanently invisible.
        intent.cc_watermark_ms = watermark;
        intent.wake_on_cc = true;
        daemon.intent_store().write_atomic(&intent).expect("seed");
        seeded.push(intent);
    }

    let report = daemon.reconcile_once().await;
    assert_eq!(report.restored, 3, "every binding must be restored exactly");

    for intent in &seeded {
        let status = match daemon
            .request(Request::Status {
                store_key: Some(intent.store_key.clone()),
                detail: true,
                proof: Some(daemon.admin_cap().to_string()),
            })
            .await
        {
            Response::StatusReport { status } => status,
            other => panic!("expected status, got {other:?}"),
        };
        let member = status
            .members
            .iter()
            .find(|m| m.store_key == intent.store_key && m.address == intent.address)
            .unwrap_or_else(|| panic!("member for {} / {}", intent.store_key, intent.address));
        assert!(member.push_registered);
        assert!(member.push_wake_on_cc);
        assert_eq!(
            member.push_cc_after_ms, intent.cc_watermark_ms,
            "the CC watermark must be preserved, not recomputed as `now`"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Bounded scanning: T27
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn station_intent_over_budget_scope_is_bounded_complete_and_never_pruned() {
    register_test_handler_kind();
    let daemon = TestDaemon::new("intent-overcap");
    let store_key = daemon.store_key("overcap");
    let _ = daemon.open_backend(&store_key).await;
    let producer_dir = daemon.root().join("producer");
    // Deliberately NOT pre-created: `ensure_owner_private_producer_root` creates a fresh root
    // with the owner-only descriptor, and *validates* (never rewrites) one that already exists.
    // A root created by an ordinary `mkdir` under a broadly-ACLed tree fails closed, which is
    // the intended posture.
    let root_id = format!("overcap_root_{}", std::process::id());
    let root = register_test_producer_root(&root_id, &producer_dir);
    let secret = "o".repeat(64);
    // Deliberately dead: this test is about *scheduling*, so every intent should take the same
    // cheap failure path rather than creating 600 members.
    let producer =
        FakeProducer::start(&root, "sess-overcap", &secret, ProducerBehavior::Healthy).await;
    producer.kill();
    let credential = root.join("sess-overcap.json");
    write_credential_file(&credential, &secret);

    let total = 600usize;
    let store = daemon.intent_store();
    for i in 0..total {
        let intent = live_intent(
            &store_key,
            "sess-overcap",
            &format!("addr:{i:04}"),
            daemon.singleton_hash(),
            &producer,
            &root_id,
            &credential,
        );
        // Bypass the write cap the way an older build or a manual copy would, so the scan path is
        // exercised above the cap rather than the write path being tested twice.
        let bytes = serde_json::to_vec_pretty(&intent).expect("encode");
        telex::platform_fs::write_owner_only_file_atomic(&store.path_for(&intent.id()), &bytes)
            .expect("seed");
    }
    assert!(total > STATION_INTENT_MAX_COUNT);

    let mut attempted = 0usize;
    let mut passes = 0usize;
    // The published ceiling: ceil(N / RECONCILE_MAX_CONCURRENCY) passes. Healthy passes drain a
    // full budget, so this converges far sooner; the loop bound is the guarantee, not the target.
    let ceiling = total.div_ceil(4);
    // Coverage is counted per binding, not as a sum of per-pass `scanned`. Above the pass budget a
    // scope's *discovery* is bounded too, so a pass sees a window rather than the whole scope and
    // may legitimately re-attempt an entry the previous window ended on; a sum reaches `total`
    // while the tail is still untouched. The property is that the round-robin cursor reaches every
    // distinct intent within the ceiling, and that is what this drives to.
    let distinct_attempted = |daemon: &TestDaemon| {
        daemon
            .intent_index()
            .entries
            .values()
            .filter(|entry| entry.attempts > 0)
            .count()
    };
    while distinct_attempted(&daemon) < total && passes < ceiling {
        let report = daemon.reconcile_once().await;
        assert!(report.over_cap, "an over-cap scope must report over_cap");
        assert_eq!(report.observed_count, total);
        assert!(
            report.scanned <= 64,
            "a pass must not exceed the per-pass budget, got {}",
            report.scanned
        );
        attempted += report.scanned;
        passes += 1;
    }
    assert!(
        attempted >= total,
        "the round-robin cursor must reach every intent within the published ceiling ({attempted} of {total} in {passes} passes)"
    );
    // `attempted` sums *attempts*, not distinct intents: a regression where the cursor stalls and
    // the same 64 intents are re-attempted every pass satisfies the sum while the tail starves.
    // The index is keyed per binding, so counting entries that were actually attempted is the
    // assertion the row is really about.
    assert_eq!(
        distinct_attempted(&daemon),
        total,
        "every distinct intent must be attempted, not the same 64 repeatedly"
    );
    assert_eq!(
        store.list_ids().expect("list").len(),
        total,
        "nothing may be deleted for being over cap"
    );
}

// ---------------------------------------------------------------------------------------------
// Version skew: T18, T19, T20
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn station_intent_schema_skew_is_surfaced_not_acted_on() {
    // T19: an intent from a newer build is incompatible, never guessed at.
    let scenario = Scenario::new("intent-skew-newer", ProducerBehavior::Healthy).await;
    let raw = std::fs::read_to_string(
        scenario
            .daemon
            .intent_store()
            .path_for(&scenario.intent.id()),
    )
    .expect("read intent");
    let mut doc: serde_json::Value = serde_json::from_str(&raw).expect("parse intent");
    doc["schema_version"] = serde_json::json!(99);
    let _ = std::fs::remove_file(
        scenario
            .daemon
            .intent_store()
            .path_for(&scenario.intent.id()),
    );
    telex::platform_fs::write_owner_only_file_atomic(
        &scenario
            .daemon
            .intent_store()
            .path_for(&scenario.intent.id()),
        serde_json::to_vec(&doc).expect("encode").as_slice(),
    )
    .expect("write skewed intent");

    let report = scenario.daemon.reconcile_once().await;
    assert_eq!(report.restored, 0);
    assert_eq!(
        report.inert, 1,
        "an unsupported schema is inert, not failed"
    );
    assert!(!scenario.member_push_registered().await);
}

#[tokio::test]
async fn station_intent_scope_is_namespaced_per_protocol_major() {
    // T20: the intent scope hashes the protocol major, so a major change cannot make two daemons
    // fight over the same intents.
    let current = TestDaemon::new("intent-major-current");
    let other =
        TestDaemon::with_protocol("intent-major-other", telex::daemon_ipc::PROTOCOL_MAJOR + 1);
    assert_ne!(
        current.singleton_hash(),
        other.singleton_hash(),
        "a protocol-major change must produce a different intent scope"
    );
    assert_ne!(
        current.intent_store().root(),
        other.intent_store().root(),
        "the two scopes must be different directories"
    );
}

// ---------------------------------------------------------------------------------------------
// The two-level API contract
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn station_intent_inline_reconcile_completes_under_a_held_admission_guard() {
    // The inline anti-downgrade path calls the guard-free inner routine while `register_member`
    // already holds the per-station admission guard. Calling the acquiring entry point there would
    // self-deadlock, so this exercises exactly that shape with a timeout.
    let scenario = Scenario::new("intent-inline-guard", ProducerBehavior::Healthy).await;
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        scenario
            .daemon
            .reconcile_intent_under_admission_guard(&scenario.intent),
    )
    .await
    .expect("the inline reconcile path must not deadlock under a held admission guard");
    assert!(
        outcome.contains("Restored") || outcome.contains("RefreshedNoOp"),
        "unexpected inline outcome: {outcome}"
    );
}

// ---------------------------------------------------------------------------------------------
// Drain and diagnostics
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn station_intent_drain_report_prefers_the_index_and_never_reconciles() {
    let scenario = Scenario::new("intent-drain", ProducerBehavior::Healthy).await;
    scenario.daemon.reconcile_once().await;

    let report = scenario.daemon.drain_intent_report();
    assert_eq!(report.recoverable, 1);
    assert_eq!(report.degraded, 0);
    assert_eq!(report.incompatible, 0);
    assert!(
        report.index_as_of_ms > 0,
        "the report must carry its own staleness"
    );

    // Removing the manifest must not change the report. The durable read only *adds* bindings the
    // index has no fresh answer for; it never retracts what a real pass proved, so a scope that
    // vanished under a running daemon is still reported from what that daemon knows.
    let path = scenario
        .daemon
        .intent_store()
        .path_for(&scenario.intent.id());
    std::fs::remove_file(&path).expect("remove manifest");
    let after = scenario.daemon.drain_intent_report();
    assert_eq!(
        after.recoverable, report.recoverable,
        "the cached projection is not retracted by a durable read"
    );

    // Draining suppresses reconciliation entirely.
    let (_, action) = scenario
        .daemon
        .request_with_action(Request::Drain {
            proof: Some(scenario.daemon.admin_cap().to_string()),
        })
        .await;
    assert_eq!(action, telex::daemon::test_support::TestClientAction::Drain);
    let suppressed = scenario.daemon.reconcile_once().await;
    assert_eq!(
        suppressed.scanned, 0,
        "a draining daemon must not arm new delivery"
    );
}

#[tokio::test]
async fn station_intent_status_projects_intent_only_rows_without_secrets() {
    let scenario = Scenario::new("intent-status", ProducerBehavior::Healthy).await;
    // Seed the index without creating a member: kill the producer so the pass records state but
    // restores nothing.
    scenario.producer.kill();
    tokio::time::sleep(Duration::from_millis(50)).await;
    scenario.daemon.reconcile_once().await;

    let status = scenario.status().await;
    let row = status
        .intents
        .iter()
        .find(|row| row.address == scenario.intent.address)
        .expect("an intent-only status row");
    assert!(
        !row.has_member,
        "the projection must distinguish an intent with no member"
    );
    assert!(row.attempts >= 1);
    assert!(status.intent_index_as_of_ms.is_some());

    // Redaction: neither the secret nor the argv may appear anywhere in the projection.
    let encoded = serde_json::to_string(&status).expect("encode status");
    assert!(!encoded.contains(&"s".repeat(64)), "status leaked a secret");
    assert!(
        !encoded.contains("--daemon-instance"),
        "status leaked raw handler argv"
    );

    // The reconcile event log exists, is NDJSON, and carries no secret either.
    let log = scenario
        .daemon
        .intent_store()
        .root()
        .join("reconcile-events.ndjson");
    let contents = std::fs::read_to_string(&log).expect("reconcile event log");
    assert!(contents.lines().count() >= 1);
    for line in contents.lines() {
        serde_json::from_str::<serde_json::Value>(line).expect("each event log line is JSON");
    }
    assert!(
        !contents.contains(&"s".repeat(64)),
        "event log leaked a secret"
    );
}

#[tokio::test]
async fn station_intent_session_end_revokes_so_an_ended_session_cannot_return() {
    let scenario = Scenario::new("intent-session-end", ProducerBehavior::Healthy).await;
    scenario.daemon.reconcile_once().await;
    assert!(scenario.member_push_registered().await);

    let ended = scenario
        .daemon
        .request(Request::SessionEnd {
            store_key: scenario.store_key.clone(),
            session_id: scenario.intent.session_id.clone(),
            proof: Some(scenario.daemon.admin_cap().to_string()),
        })
        .await;
    assert!(matches!(ended, Response::Ack { .. }), "{ended:?}");

    let stored = scenario
        .daemon
        .intent_store()
        .load(&scenario.intent.id())
        .expect("intent still present");
    assert_eq!(
        stored.state,
        IntentRecoveryState::Revoked,
        "an ended session must never be re-attended by a stale intent"
    );

    let report = scenario.daemon.reconcile_once().await;
    assert_eq!(report.restored, 0);
}

#[tokio::test]
async fn station_intent_reconcile_request_requires_an_admin_proof() {
    let scenario = Scenario::new("intent-proof", ProducerBehavior::Healthy).await;
    let unproofed = scenario
        .daemon
        .request(Request::ReconcileIntents {
            proof: None,
            scope: None,
        })
        .await;
    match unproofed {
        Response::Error { code, .. } => {
            assert_eq!(code, telex::daemon_ipc::ERROR_UNAUTHORIZED)
        }
        other => panic!("an arming operation must not be reachable unproofed: {other:?}"),
    }

    let proofed = scenario
        .daemon
        .request(Request::ReconcileIntents {
            proof: Some(scenario.daemon.admin_cap().to_string()),
            scope: Some(scenario.store_key.clone()),
        })
        .await;
    match proofed {
        Response::Reconciled { report } => assert_eq!(report.restored, 1),
        other => panic!("expected a reconcile report, got {other:?}"),
    }
}

#[tokio::test]
async fn station_intent_trigger_seam_drives_a_pass_without_a_wall_clock_sleep() {
    let scenario = Scenario::new("intent-trigger", ProducerBehavior::Healthy).await;
    // Wire the trigger half of the production heartbeat loop. Without a consumer the pulse has
    // nowhere to go, the await times out, and the test's fallback (`reconcile_once` directly)
    // asserts nothing about the seam at all — while costing the full timeout to say so.
    let consumer = scenario.daemon.spawn_trigger_consumer();
    let mut reports = scenario.daemon.reconcile_reports();
    let before = reports.borrow_and_update().pass_seq;
    let report = scenario
        .daemon
        .pulse_reconcile_and_wait(Duration::from_secs(10))
        .await
        .expect("a pulse on the trigger seam must drive a pass and publish its report");
    assert!(report.pass_seq > before);
    assert!(report.ran, "a pulse-driven pass must actually run");
    assert_eq!(
        report.restored, 1,
        "the pulse-driven pass must do the same work a tick-driven pass does"
    );
    let observed = reports.borrow_and_update().clone();
    assert!(
        observed.pass_seq >= report.pass_seq,
        "every completed pass must be published on the report seam"
    );
    consumer.abort();
}

/// A pass that did not run must be distinguishable from one that ran and found nothing.
///
/// This is what `telex upgrade`'s successor verification and the `agentStop` drain hook rely on:
/// both previously printed `restored 0` for a suppressed pass, reporting a completed verification
/// for a recovery that never started.
#[tokio::test]
async fn station_intent_a_suppressed_pass_is_reported_as_not_run() {
    let scenario = Scenario::new("intent-not-run", ProducerBehavior::Healthy).await;
    let ran = scenario.daemon.reconcile_once().await;
    assert!(ran.ran, "an ordinary pass reports that it ran");
    assert_eq!(ran.skipped_reason, None);

    let (_drain, action) = scenario
        .daemon
        .request_with_action(Request::Drain {
            proof: Some(scenario.daemon.admin_cap().to_string()),
        })
        .await;
    assert_eq!(action, telex::daemon::test_support::TestClientAction::Drain);
    let suppressed = scenario.daemon.reconcile_once().await;
    assert!(
        !suppressed.ran,
        "a drain-suppressed pass must not claim to have run"
    );
    assert_eq!(suppressed.skipped_reason.as_deref(), Some("draining"));
    assert_eq!(suppressed.restored, 0);
    assert!(
        suppressed.pass_seq > ran.pass_seq,
        "the pass sequence still advances, which is exactly why `ran` is needed"
    );
}

#[tokio::test]
async fn station_intent_pending_intent_is_reported_as_needing_attach_not_silently_retried() {
    // A `pending` intent means a push attach is mid-flight (or crashed before finalizing). The
    // daemon never acts on a pending intent, so a generic re-register-and-retry would race the
    // attach; the typed reason is what lets the client stop and point at the finalizing step.
    let scenario = Scenario::new("intent-pending", ProducerBehavior::Healthy).await;
    let mut pending = scenario.intent.clone();
    pending.state = IntentRecoveryState::Pending;
    pending.generation = 2;
    scenario.reseed(&pending);

    // A pending intent is never reconciled.
    let report = scenario.daemon.reconcile_once().await;
    assert_eq!(report.restored, 0);
    assert_eq!(report.inert, 1);
    assert!(!scenario.member_push_registered().await);

    // And a wait against the unattended station names the specific reason.
    let response = scenario
        .daemon
        .request(Request::Wait {
            store_key: scenario.store_key.clone(),
            session_id: scenario.intent.session_id.clone(),
            address: scenario.intent.address.clone(),
            attention: None,
            min_attention: None,
            wake_on_cc: false,
            timeout_ms: Some(50),
            waiter_pid: Some(std::process::id()),
            waiter_start_time: None,
        })
        .await;
    match response {
        Response::Error {
            code,
            needs_attach_reason,
            ..
        } => {
            assert_eq!(code, telex::daemon_ipc::ERROR_NEEDS_ATTACH);
            assert_eq!(
                needs_attach_reason,
                Some(NeedsAttachReason::PushIntentPending),
                "a pending push attach must be named, not reported as generic restart loss"
            );
        }
        other => panic!("expected a typed needs-attach, got {other:?}"),
    }
}

#[tokio::test]
async fn station_intent_cursor_advances_only_past_attempted_intents() {
    // Regression guard for a silent starvation bug: if the scan cursor advances to the last entry
    // it *loaded* rather than the last the pass *attempted*, then a pass truncated by the deadline
    // skips everything in between, permanently, and the tail of a scope never recovers.
    let run_dir = std::env::temp_dir().join(format!(
        "telex-cursor-fairness-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&run_dir).expect("run dir");
    let store = telex::station_intent::IntentStore::open(&run_dir, "cursorhash").expect("store");

    // Seed a small scope — smaller than any plausible pass budget, which is the case the naive
    // implementation got wrong (the cursor pinned to the maximum and `start` reset to 0 forever).
    let producer_dir = run_dir.join("producer");
    let root_id = format!("cursor_root_{}", std::process::id());
    let root = register_test_producer_root(&root_id, &producer_dir);
    let secret = "c".repeat(64);
    let producer =
        FakeProducer::start(&root, "cursor-session", &secret, ProducerBehavior::Healthy).await;
    let credential = root.join("cursor-session.json");
    write_credential_file(&credential, &secret);
    for i in 0..6 {
        let mut intent = live_intent(
            "sqlite:/cursor",
            "cursor-session",
            &format!("addr:{i:02}"),
            "cursorhash",
            &producer,
            &root_id,
            &credential,
        );
        intent.generation = 1;
        store.write_atomic(&intent).expect("seed");
    }

    // A pass that attempts only two entries must resume at the third, not at the seventh.
    let page = store.scan(6).expect("scan");
    assert_eq!(page.loaded.len(), 6);
    assert_eq!(page.loaded_positions.len(), 6);
    let first_two: Vec<String> = page.loaded[..2].iter().map(|i| i.address.clone()).collect();
    store
        .advance_cursor(&page.loaded_positions[1])
        .expect("advance to the last attempted entry");

    let next = store.scan(6).expect("second scan");
    assert_eq!(
        next.loaded[0].address, page.loaded[2].address,
        "the next pass must resume at the first entry the previous pass did not attempt"
    );
    assert!(
        !first_two.contains(&next.loaded[0].address),
        "the next pass must not restart at an already-attempted entry"
    );

    // Sweeping the whole scope wraps back to the beginning, so coverage is cyclic, not one-shot.
    store
        .advance_cursor(&page.loaded_positions[5])
        .expect("advance past the maximum sort position");
    let wrapped = store.scan(6).expect("third scan");
    assert_eq!(
        wrapped.loaded[0].address, page.loaded[0].address,
        "the cursor must wrap after a complete sweep"
    );

    let _ = std::fs::remove_dir_all(&run_dir);
}

#[tokio::test]
async fn station_intent_generation_never_resets_so_a_stale_pass_cannot_clobber_a_resume() {
    // A generation that cycles 1 -> 2 -> 1 -> 2 defeats the write-CAS: a pass that read generation 2
    // could write back over a *newer* manifest that had cycled to 2 again, restoring a stale
    // producer descriptor and permanently breaking recovery for that station.
    let scenario = Scenario::new("intent-generation", ProducerBehavior::Healthy).await;
    let store = scenario.daemon.intent_store();
    let id = scenario.intent.id();
    let first = store.load(&id).expect("seeded intent").generation;

    // Model a resume: the pending write must build on the existing generation, not reset it.
    let mut resumed = scenario.intent.clone();
    resumed.state = IntentRecoveryState::Pending;
    resumed.generation = first.saturating_add(1);
    store.write_atomic(&resumed).expect("pending rewrite");
    let mut finalized = resumed.clone();
    finalized.state = IntentRecoveryState::Live;
    finalized.generation = resumed.generation.saturating_add(1);
    store.write_atomic(&finalized).expect("finalize");

    let observed = store.load(&id).expect("reload").generation;
    assert!(
        observed > first,
        "generation must be monotonic across a resume cycle ({observed} must exceed {first})"
    );

    // And a CAS from the pre-resume generation must lose.
    let mut stale = scenario.intent.clone();
    stale.occupant = "stale-pass".to_string();
    stale.generation = first;
    assert!(
        !store.write_cas(first, &stale).expect("cas"),
        "a pass holding a pre-resume generation must not be able to write back"
    );
    assert_eq!(
        store.load(&id).expect("reload").occupant,
        scenario.intent.occupant,
        "the fresh manifest must survive a stale pass"
    );
}

#[tokio::test]
async fn station_intent_anti_downgrade_guard_works_before_the_first_reconcile_pass() {
    // The index is populated by a reconcile pass, and `serve()` accepts connections before that
    // pass runs — which is exactly the daemon-replacement window the guard exists to protect. The
    // guard must therefore consult the durable manifest, not only the cache.
    let scenario = Scenario::new("intent-cold-guard", ProducerBehavior::Healthy).await;
    assert!(
        scenario.daemon.intent_index().entries.is_empty(),
        "precondition: no pass has run, so the cached index is empty"
    );

    let response = scenario
        .daemon
        .request(register_request(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address,
        ))
        .await;
    assert!(
        matches!(response, Response::Registered { .. }),
        "a recoverable intent should reconcile inline: {response:?}"
    );
    assert!(
        scenario.member_push_registered().await,
        "a pull-only Register before the first pass must still not downgrade a live push intent"
    );
    let drain = scenario.daemon.drain_intent_report();
    assert_eq!(
        drain.recoverable, 1,
        "the inline restore must update the cached projection before an immediate drain"
    );
    assert_eq!(
        drain.degraded, 0,
        "an inline success must clear stale failure projection"
    );
}

// ---------------------------------------------------------------------------------------------
// Regressions from the final adversarial review pass
// ---------------------------------------------------------------------------------------------

/// A transient credential condition must take the failure ladder, not the one-hour quarantine
/// cadence.
///
/// The credential is the bridge registry: rewritten on a 15 s heartbeat, deleted and recreated on
/// every `/clear` or `extensions_reload`, and read by the reconciler every 5 s. Classifying a
/// stale mtime (or a truncated read, or a missing field) as `Terminal` parked the binding for an
/// hour with nothing to shorten the wait, replacing the published recovery bounds with a wedge.
#[tokio::test]
async fn station_intent_transient_credential_conditions_take_the_backoff_ladder() {
    let stale = Scenario::new("intent-cred-ladder", ProducerBehavior::Healthy).await;
    let mut aged = stale.intent.clone();
    aged.producer.credential.max_age_ms = 1;
    aged.generation = 2;
    stale.reseed(&aged);
    tokio::time::sleep(Duration::from_millis(25)).await;

    let report = stale.daemon.reconcile_once().await;
    assert_eq!(
        report.failed, 1,
        "a stale credential is a retryable failure, not an inert terminal state"
    );
    assert_eq!(stale.failure_code().as_deref(), Some("credential_stale"));
    assert_eq!(
        stale.intent_state(),
        IntentRecoveryState::Unverifiable,
        "the projected state stays Unverifiable: retry policy and projection are separate axes"
    );

    let entry = stale
        .daemon
        .intent_index()
        .entries
        .values()
        .next()
        .cloned()
        .expect("an index entry");
    assert_eq!(
        entry.consecutive_failures, 1,
        "a transient condition must climb the ladder so it can climb back down"
    );
    let next = entry.next_attempt_ms.expect("a scheduled next attempt");
    let delay = next - entry.last_attempt_ms.expect("a last attempt");
    assert!(
        delay < 60_000,
        "the next attempt must be on the backoff ladder, not the one-hour quarantine cadence \
         (got {delay} ms)"
    );
}

/// A restore must never rewind a live member's CC watermark, and the durable watermark must be
/// refreshed so a later restore replays only the outage window.
#[tokio::test]
async fn station_intent_cc_watermark_is_refreshed_and_never_rewound() {
    let scenario = Scenario::new("intent-cc-watermark", ProducerBehavior::Healthy).await;
    let mut wake = scenario.intent.clone();
    wake.wake_on_cc = true;
    wake.cc_watermark_ms = Some(1_000);
    wake.generation = 2;
    scenario.reseed(&wake);

    scenario.daemon.reconcile_once().await;
    assert!(scenario.member_push_registered().await);

    // The member advances its watermark in memory as CC traffic is accepted.
    scenario.daemon.set_member_cc_after_ms(
        &scenario.store_key,
        &scenario.intent.session_id,
        &scenario.intent.address,
        Some(9_000),
    );
    scenario.daemon.reconcile_once().await;

    let persisted = scenario
        .daemon
        .intent_store()
        .load(&scenario.intent.id())
        .expect("reload the manifest");
    assert_eq!(
        persisted.cc_watermark_ms,
        Some(9_000),
        "the durable watermark must track the live member, or every daemon replacement replays \
         the whole session's CC history"
    );

    // And a restore over a member that has advanced further must not lower it.
    let mut behind = persisted.clone();
    behind.cc_watermark_ms = Some(2_000);
    behind.generation = persisted.generation + 1;
    scenario.reseed(&behind);
    scenario.daemon.forget_member(
        &scenario.store_key,
        &scenario.intent.session_id,
        &scenario.intent.address,
    );
    scenario.daemon.reconcile_once().await;
    let restored = scenario
        .daemon
        .member_cc_after_ms(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address,
        )
        .expect("a restored member");
    assert_eq!(
        restored,
        Some(2_000),
        "with no live member to compare against, the manifest floor is what keeps gap-committed \
         CC messages visible"
    );
}

/// `telex station reset` is the one deliberate operator action with no durable tombstone, so it
/// was the one the reconciler could not see: the next pass re-registered the member within a tick.
#[tokio::test]
async fn station_intent_operator_reset_is_not_undone_by_the_reconciler() {
    let scenario = Scenario::new("intent-reset", ProducerBehavior::Healthy).await;
    scenario.daemon.reconcile_once().await;
    assert!(scenario.member_push_registered().await);

    let reset = scenario
        .daemon
        .request(Request::Reset {
            store_key: scenario.store_key.clone(),
            address: scenario.intent.address.clone(),
            proof: Some(scenario.daemon.admin_cap().to_string()),
        })
        .await;
    assert!(matches!(reset, Response::Ack { .. }), "{reset:?}");
    let idle_after_reset = scenario
        .status()
        .await
        .members
        .iter()
        .any(|m| m.address == scenario.intent.address && m.idle);
    assert!(
        idle_after_reset,
        "precondition: reset marks the member idle"
    );

    for _ in 0..3 {
        let report = scenario.daemon.reconcile_once().await;
        assert_eq!(
            report.restored, 0,
            "the reconciler must not re-arm a station the operator reset"
        );
        let still_idle = scenario
            .status()
            .await
            .members
            .iter()
            .any(|m| m.address == scenario.intent.address && m.idle);
        assert!(
            still_idle,
            "a reset station must stay idle until an explicit resume"
        );
    }
    let persisted = scenario
        .daemon
        .intent_store()
        .load(&scenario.intent.id())
        .expect("reload the manifest");
    assert_eq!(
        persisted.state,
        IntentRecoveryState::Revoked,
        "reset withdraws the desired state durably, so it survives a daemon replacement too"
    );
}

/// `Pending` is never reconciled, so counting it as `recoverable` told the operator that bindings
/// would come back automatically when they cannot, and made `telex upgrade` wait out its
/// successor timeout for a pass that could restore nothing.
#[tokio::test]
async fn station_intent_drain_report_counts_pending_separately_from_recoverable() {
    let scenario = Scenario::new("intent-drain-pending", ProducerBehavior::Healthy).await;
    let mut pending = scenario.intent.clone();
    pending.state = IntentRecoveryState::Pending;
    pending.generation = 2;
    scenario.reseed(&pending);
    scenario.daemon.reconcile_once().await;

    let report = scenario.daemon.drain_intent_report();
    assert_eq!(
        report.recoverable, 0,
        "a pending intent is not something a successor restores automatically"
    );
    assert_eq!(report.pending, 1, "but it must still be visible");
}

/// A manifest rejected by a schema or descriptor check must still reach the index whenever the
/// binding it names can be read, or `telex status` and the drain report show nothing at all for
/// exactly the intents they exist to flag.
#[tokio::test]
async fn station_intent_rejected_manifests_are_visible_in_status_and_the_drain_report() {
    let scenario = Scenario::new("intent-rejected-visible", ProducerBehavior::Healthy).await;
    let path = scenario
        .daemon
        .intent_store()
        .path_for(&scenario.intent.id());
    let raw = std::fs::read_to_string(&path).expect("read intent");
    let mut doc: serde_json::Value = serde_json::from_str(&raw).expect("parse intent");
    doc["schema_version"] = serde_json::json!(99);
    let _ = std::fs::remove_file(&path);
    telex::platform_fs::write_owner_only_file_atomic(
        &path,
        serde_json::to_vec(&doc).expect("encode").as_slice(),
    )
    .expect("write skewed intent");

    scenario.daemon.reconcile_once().await;
    let row = scenario
        .daemon
        .intent_statuses()
        .into_iter()
        .find(|row| row.address == scenario.intent.address)
        .expect("a rejected manifest must still produce a status row");
    assert_eq!(row.state, IntentRecoveryState::Incompatible);
    let drain = scenario.daemon.drain_intent_report();
    assert_eq!(
        drain.incompatible, 1,
        "the drain report must not report zero for an intent this build cannot reconcile"
    );
}

/// Backoff and quarantine live in the in-memory index, which starts empty on every daemon start —
/// the event most likely to follow a crash loop. The durable evidence block is what carries them
/// across, and it was written every pass and never read back.
#[tokio::test]
async fn station_intent_retry_state_survives_a_daemon_replacement() {
    let scenario = Scenario::new("intent-evidence", ProducerBehavior::WrongNonce).await;
    scenario.daemon.reconcile_once().await;
    let persisted = scenario
        .daemon
        .intent_store()
        .load(&scenario.intent.id())
        .expect("reload the manifest");
    assert_eq!(
        persisted.evidence.consecutive_failures, 1,
        "a genuine failure must be recorded durably"
    );
    assert!(persisted.evidence.next_attempt_ms.is_some());

    // A successor daemon sees the same scope for the first time.
    scenario.daemon.clear_intent_index();
    scenario.daemon.reconcile_once().await;
    let entry = scenario
        .daemon
        .intent_index()
        .entries
        .values()
        .next()
        .cloned()
        .expect("an index entry");
    assert!(
        entry.consecutive_failures >= 1,
        "a successor must not hand a wedged intent a fresh full-rate retry budget"
    );
}

/// T15: two sessions holding a live intent for one address must never both end up with an armed
/// member. The per-address dedupe was keyed on the full `(store, session, address)` tuple, which
/// no two manifests can ever share, so it rejected nothing; and the `AlreadyOwned` adoption branch
/// treated "this daemon already owns the address" as licence to adopt the lease for a second
/// session.
///
/// Asserted as **exactly one, and the expected one**. `<= 1` passed for the two failures that
/// matter most — zero armed members (nothing recovered at all) and the wrong session winning — so
/// it could not distinguish the property from its own absence. Both rivals get a *healthy*
/// producer and their own credential here, so nothing but the ordering rule decides the winner:
/// scan order is `(store_key, address, generation desc)`, so the higher generation wins.
#[tokio::test]
async fn station_intent_two_sessions_never_both_attend_one_address() {
    let first = Scenario::with_session(
        "intent-competing",
        ProducerBehavior::Healthy,
        "sess-a",
        "addr:contested",
    )
    .await;
    // The incumbent, at a decisively higher generation than its rival.
    let mut incumbent = first.intent.clone();
    incumbent.generation = 5;
    first.reseed(&incumbent);

    // A second session's live intent for the same address, in the same store and scope, with a
    // producer of its own that answers correctly for `sess-b`. Without that, the rival would lose
    // on `probe_session_mismatch` and the test would pass without the dedupe existing at all.
    let root = first
        .credential_path
        .parent()
        .expect("credential root")
        .to_path_buf();
    let rival_secret = "r".repeat(64);
    let rival_producer =
        FakeProducer::start(&root, "sess-b", &rival_secret, ProducerBehavior::Healthy).await;
    let rival_credential = root.join("sess-b.json");
    write_credential_file(&rival_credential, &rival_secret);
    let mut rival = live_intent(
        &first.store_key,
        "sess-b",
        "addr:contested",
        first.daemon.singleton_hash(),
        &rival_producer,
        first.intent.producer.credential.root_id.as_str(),
        &rival_credential,
    );
    rival.generation = 1;
    first.reseed(&rival);

    for _ in 0..3 {
        first.daemon.reconcile_once().await;
    }
    let armed: Vec<String> = first
        .status()
        .await
        .members
        .iter()
        .filter(|m| m.address == "addr:contested" && m.push_registered && !m.idle)
        .map(|m| m.session_id.clone())
        .collect();
    assert_eq!(
        armed,
        vec!["sess-a".to_string()],
        "exactly one session must attend a contested address, and it must be the \
         highest-generation intent the deterministic scan order names first"
    );
    // The loser is not silently dropped: it stays visible so an operator can see the conflict.
    let rows = first.daemon.intent_statuses();
    assert!(
        rows.iter()
            .any(|row| row.session_id == "sess-b" && row.address == "addr:contested"),
        "the losing intent must still be indexed, got {rows:?}"
    );
}

/// An **inert** record must never consume the per-address winner slot.
///
/// The dedupe used to run before the `is_reconcilable` check, so the highest-generation manifest
/// for an address claimed the slot whatever its state. A `revoked` generation 3 for one session
/// therefore shadowed a `live` generation 2 for another — and not merely as a scheduling delay:
/// the shadowed record was `continue`d *before* it was indexed, so for as long as the tombstone
/// survived (up to its seven-day TTL) the live binding was invisible to `telex status`, absent
/// from the pre-drain report, unseen by the turn guard, and attempted by no pass at all.
#[tokio::test]
async fn station_intent_a_revoked_record_never_shadows_a_live_one_for_the_same_address() {
    let live = Scenario::with_session(
        "intent-shadowed",
        ProducerBehavior::Healthy,
        "sess-live",
        "addr:shadowed",
    )
    .await;
    let mut current = live.intent.clone();
    current.generation = 2;
    live.reseed(&current);

    // A *higher*-generation tombstone for the same address, left by another session's detach.
    let mut revoked = live.intent.clone();
    revoked.session_id = "sess-gone".to_string();
    revoked.handler.session_id = "sess-gone".to_string();
    revoked.state = IntentRecoveryState::Revoked;
    revoked.generation = 3;
    live.reseed(&revoked);

    let report = live.daemon.reconcile_once().await;
    assert_eq!(
        report.restored, 1,
        "the live record must be attempted, not shadowed by the tombstone"
    );
    assert_eq!(
        report.inert, 1,
        "the tombstone is inert, and counted as such"
    );

    let armed: Vec<String> = live
        .status()
        .await
        .members
        .iter()
        .filter(|m| m.address == "addr:shadowed" && m.push_registered && !m.idle)
        .map(|m| m.session_id.clone())
        .collect();
    assert_eq!(
        armed,
        vec!["sess-live".to_string()],
        "exactly one armed member, and it must be the live intent's session"
    );

    // Both records reach the index: the live one so status/drain/guard can see it, the tombstone
    // so an operator can see why the address looks contested.
    let rows = live.daemon.intent_statuses();
    let live_row = rows
        .iter()
        .find(|row| row.session_id == "sess-live")
        .expect("the live intent must be indexed");
    assert_eq!(live_row.state, IntentRecoveryState::Restored);
    let revoked_row = rows
        .iter()
        .find(|row| row.session_id == "sess-gone")
        .expect("the revoked intent must be indexed");
    assert_eq!(revoked_row.state, IntentRecoveryState::Revoked);

    let drain = live.daemon.drain_intent_report();
    assert_eq!(
        drain.recoverable, 1,
        "the drain report must see the live binding a successor will restore"
    );
}

/// The failure ladder is earned by a *producer descriptor*, not by a binding, and a durable state
/// transition replaces the descriptor. Carrying the ladder across one means the repair for a
/// reload is followed by a wait the repaired record did not earn — and past
/// `RECONCILE_QUARANTINE_AFTER`, an hour of it, which is long enough that "recovery is automatic"
/// stops being true in practice. An *evidence* rewrite is deliberately not a transition (it writes
/// under the same generation), so it must not clear anything.
#[tokio::test]
async fn station_intent_a_durable_transition_clears_a_ladder_the_old_descriptor_earned() {
    let scenario = Scenario::new("intent-ladder-reset", ProducerBehavior::WrongNonce).await;
    for _ in 0..3 {
        scenario.daemon.reconcile_once().await;
    }
    let wedged = scenario
        .daemon
        .intent_index()
        .entries
        .values()
        .next()
        .cloned()
        .expect("an index entry");
    assert!(
        wedged.consecutive_failures >= 1,
        "precondition: the intent is on the ladder"
    );
    assert!(wedged.next_attempt_ms.is_some());

    // A pass that only rewrites evidence keeps the ladder: the record did not change.
    scenario.daemon.reconcile_once().await;
    let still_wedged = scenario
        .daemon
        .intent_index()
        .entries
        .values()
        .next()
        .cloned()
        .expect("an index entry");
    assert!(
        still_wedged.consecutive_failures >= wedged.consecutive_failures,
        "an evidence rewrite is not a state transition and must not forgive failures"
    );

    // A durable transition — exactly what a turn-boundary producer-identity refresh performs —
    // does clear it, and the very next pass attempts the binding again.
    scenario
        .daemon
        .intent_store()
        .update_locked(&scenario.intent.id(), |intent| {
            intent.updated_at_ms += 1;
            true
        })
        .expect("transition")
        .expect("the record must still exist");
    let report = scenario.daemon.reconcile_once().await;
    assert_eq!(
        report.scanned, 1,
        "the transition must re-admit the binding to the fast cadence instead of leaving it \
         parked on a schedule its previous descriptor earned"
    );
}

/// The same rule, one layer down and where it actually bit: the **durable** evidence block.
///
/// Clearing the in-memory ladder on a generation move only repairs the daemon that happened to
/// observe the reload. The durable `consecutive_failures` / `failure_code` / `next_attempt_ms`
/// are what a *successor* daemon seeds a brand-new index from on first sight, so the canonical
/// sequence — bridge reloads, passes fail `producer_identity_mismatch`, the turn-boundary hook
/// re-records the live identity, and *then* the daemon is replaced (an upgrade, a crash, the
/// restart after a crash loop) — handed the successor a repaired binding still parked on the dead
/// producer's schedule, up to a full quarantine hour. "Recovery is automatic" is only true if the
/// ladder dies with the descriptor that earned it on disk as well as in memory.
///
/// The lifetime counters are the other half of the property. Zeroing the whole evidence block
/// would pass the recovery assertion and destroy the audit trail that lets an operator tell a
/// binding that has always been broken from one that was just repaired, so `attempts` and the
/// historical timestamps are asserted to survive.
#[tokio::test]
async fn station_intent_an_identity_refresh_clears_the_durable_ladder_a_successor_seeds_from() {
    let scenario = Scenario::new("intent-identity-ladder", ProducerBehavior::Healthy).await;
    let live_producer = scenario.intent.producer.clone();
    // The bridge reloaded: the record names a process that no longer exists, so every pass fails
    // identity verification against a healthy producer that is right there.
    let mut stale = scenario.intent.clone();
    stale.producer.start_time = stale.producer.start_time.wrapping_add(1);
    scenario.reseed(&stale);

    // Two real failures, each in its own pass, so the ladder is a schedule rather than a single
    // rounding error and `attempts` is unambiguously more than one.
    for attempt in 1..=2 {
        if attempt > 1 {
            let store = scenario.daemon.intent_store();
            let mut persisted = store.load(&scenario.intent.id()).expect("load evidence");
            persisted.evidence.next_attempt_ms = None;
            assert!(
                store
                    .write_cas(persisted.generation, &persisted)
                    .expect("clear the retry delay"),
                "the test owns the manifest generation"
            );
            scenario.daemon.clear_intent_index();
        }
        let report = scenario.daemon.reconcile_once().await;
        assert_eq!(report.failed, 1, "attempt {attempt} must fail");
    }
    assert_eq!(
        scenario.failure_code().as_deref(),
        Some("producer_identity_mismatch")
    );

    let wedged = scenario
        .daemon
        .intent_store()
        .load(&scenario.intent.id())
        .expect("reload the wedged manifest");
    assert!(
        wedged.evidence.consecutive_failures >= 2,
        "precondition: the durable ladder is what a successor would inherit"
    );
    assert!(
        wedged
            .evidence
            .next_attempt_ms
            .is_some_and(|next| next > telex::model::now_ms()),
        "precondition: the durable schedule parks the binding into the future"
    );
    let attempts_before = wedged.evidence.attempts;
    let last_attempt_before = wedged.evidence.last_attempt_ms;
    assert!(
        attempts_before >= 2,
        "precondition: a lifetime history exists"
    );

    // Exactly the durable transition the turn-boundary finalize performs under the per-intent
    // write lock — the production rule itself, not a re-implementation of it.
    let identity = ProducerIdentity {
        pid: live_producer.pid,
        start_time: live_producer.start_time,
        exe_path: live_producer.exe_path.clone(),
        host_id: live_producer.host_id.clone(),
        boot_id: live_producer.boot_id.clone(),
        protocol: live_producer.protocol,
    };
    let refreshed = scenario
        .daemon
        .intent_store()
        .update_locked(&scenario.intent.id(), |intent| {
            intent.apply_producer_identity(&identity)
        })
        .expect("identity refresh")
        .expect("a real identity change must be written");
    assert_eq!(refreshed.evidence.consecutive_failures, 0);
    assert_eq!(refreshed.evidence.failure_code, None);
    assert_eq!(
        refreshed.evidence.next_attempt_ms, None,
        "the repaired record must not carry the dead descriptor's schedule on disk"
    );
    assert_eq!(
        refreshed.evidence.attempts, attempts_before,
        "lifetime counters are an audit trail, not a schedule"
    );
    assert_eq!(refreshed.evidence.last_attempt_ms, last_attempt_before);
    assert_eq!(
        refreshed.evidence.last_success_ms,
        wedged.evidence.last_success_ms
    );
    assert_eq!(
        refreshed.evidence.producer_verified_ms,
        wedged.evidence.producer_verified_ms
    );

    // The daemon that observed the reload is now replaced. The successor's index is empty, so
    // everything it decides comes from the durable block alone.
    scenario.daemon.clear_intent_index();
    let successor = scenario.daemon.reconcile_once().await;
    assert_eq!(
        successor.skipped, 0,
        "a successor must not park a repaired binding on the previous descriptor's schedule"
    );
    assert_eq!(
        successor.restored, 1,
        "the repaired binding must be restored on the successor's very first pass"
    );
    let entry = scenario
        .daemon
        .intent_index()
        .entries
        .values()
        .next()
        .cloned()
        .expect("a seeded index entry");
    assert_eq!(entry.state, IntentRecoveryState::Restored);
    assert_eq!(
        entry.consecutive_failures, 0,
        "the successor seeded a reset ladder"
    );
    assert!(
        entry.attempts > attempts_before,
        "the successor seeded the retained lifetime counter and kept counting from it"
    );
    assert!(
        scenario.member_push_registered().await,
        "push delivery must actually be armed again"
    );
}

/// A producer-side finalize is written by a *different process* than the daemon, and the pre-drain
/// report is projected from the daemon's cached index — which only a reconcile pass refreshes. So
/// `attach` immediately followed by `upgrade` drained with `recoverable = 0` for a binding that
/// had just been fully armed and finalized, and the successor-verification step skipped itself on
/// "no recoverable station intents": the one path that is supposed to carry push delivery across a
/// daemon replacement quietly became a no-op.
#[tokio::test]
async fn station_intent_a_finalize_is_visible_to_the_very_next_drain_decision() {
    let scenario = Scenario::new("intent-finalize-drain", ProducerBehavior::Healthy).await;
    let mut pending = scenario.intent.clone();
    pending.state = IntentRecoveryState::Pending;
    pending.generation = 2;
    scenario.reseed(&pending);
    scenario.daemon.reconcile_once().await;

    let before = scenario.daemon.drain_intent_report();
    assert_eq!(before.pending, 1);
    assert_eq!(
        before.recoverable, 0,
        "a pending record is not something a successor restores"
    );

    // Exactly the durable transition `finalize_intent` performs under the per-intent write lock,
    // and nothing else: no reconcile pass, no daemon involvement, no index refresh.
    scenario
        .daemon
        .intent_store()
        .update_locked(&scenario.intent.id(), |intent| {
            intent.state = IntentRecoveryState::Live;
            true
        })
        .expect("finalize")
        .expect("the record must still exist");

    let after = scenario.daemon.drain_intent_report();
    assert_eq!(
        after.recoverable, 1,
        "an upgrade started immediately after a finalize must see the binding it has to hand over"
    );
    assert_eq!(after.pending, 0);
}

/// The pre-drain backfill keeps a cached *problem* because a successor will hit the same wall — but
/// only while that problem still describes the record the successor is going to read.
///
/// The canonical sequence is the one this whole recovery path exists for: a bridge reloads, the
/// pass fails `producer_identity_mismatch` against the stale `(pid, start_time)` and caches
/// `Unverifiable` for generation N, and then the turn-boundary hook re-records the live identity at
/// generation N+1. Nothing refreshes the cached projection in between — only a reconcile pass does
/// that, and `upgrade` drains before the next tick. Holding the stale verdict made a binding that
/// had *just been repaired* drain as `degraded`, so successor verification skipped the hand-off it
/// exists to perform and push delivery was silently dropped across the replacement.
///
/// Generation is what decides applicability: durable state transitions move it, and the reconciler's
/// evidence-only rewrites deliberately do not.
#[tokio::test]
async fn station_intent_a_cached_failure_never_outlives_the_generation_it_was_recorded_against() {
    let scenario = Scenario::new("intent-drain-generation", ProducerBehavior::Healthy).await;
    // The bridge reloaded: the record still names the old process identity.
    let mut stale = scenario.intent.clone();
    stale.generation = 2;
    stale.producer.start_time = stale.producer.start_time.wrapping_add(1);
    scenario.reseed(&stale);

    let report = scenario.daemon.reconcile_once().await;
    assert_eq!(report.failed, 1);
    assert_eq!(
        scenario.failure_code().as_deref(),
        Some("producer_identity_mismatch")
    );
    let before = scenario.daemon.drain_intent_report();
    assert_eq!(
        before.degraded, 1,
        "while the cached verdict still describes the manifest on disk, it wins: a successor \
         reading generation 2 really will fail the same way"
    );
    assert_eq!(before.recoverable, 0);

    // Exactly the durable transition the turn-boundary hook performs, in the producer's process,
    // with no reconcile pass and no index refresh in between.
    scenario
        .daemon
        .intent_store()
        .update_locked(&scenario.intent.id(), |intent| {
            intent.producer.start_time = scenario.intent.producer.start_time;
            intent.state = IntentRecoveryState::Live;
            true
        })
        .expect("identity refresh")
        .expect("the record must still exist");
    let refreshed_generation = scenario
        .daemon
        .intent_store()
        .load(&scenario.intent.id())
        .expect("reload")
        .generation;
    assert!(
        refreshed_generation > 2,
        "precondition: the repair is a real durable transition"
    );
    assert_eq!(
        scenario
            .daemon
            .intent_index()
            .entries
            .values()
            .next()
            .expect("an index entry")
            .generation,
        2,
        "precondition: no pass has refreshed the cached projection"
    );

    let after = scenario.daemon.drain_intent_report();
    assert_eq!(
        after.recoverable, 1,
        "a drain started immediately after the repair must hand over the binding it repaired"
    );
    assert_eq!(
        after.degraded, 0,
        "the cached failure described a generation that no longer exists"
    );
    assert_eq!(after.pending, 0);
}

/// The daemon writes the durable **armed proof** itself, at `Register`, so a crash anywhere between
/// arming push and the producer-side finalize leaves a record that says push was armed.
#[tokio::test]
async fn station_intent_the_daemon_stamps_the_armed_proof_when_it_arms_push() {
    let scenario = Scenario::new("intent-armed-proof", ProducerBehavior::Healthy).await;
    let mut pending = scenario.intent.clone();
    pending.state = IntentRecoveryState::Pending;
    pending.generation = 2;
    scenario.reseed(&pending);

    // `replace_on_deliver` is passed explicitly rather than pinned to `true`: paired with
    // `on_deliver: None` it is the *explicit push-to-pull downgrade*, which owns the matching intent
    // withdrawal and would delete this pending fixture. An ordinary pull attach does not send it.
    let register = |on_deliver: Option<Vec<String>>, replace_on_deliver: bool| Request::Register {
        store_key: scenario.store_key.clone(),
        address: scenario.intent.address.clone(),
        session_id: scenario.intent.session_id.clone(),
        occupant: "occupant".to_string(),
        description: None,
        scope: None,
        tags: None,
        watch_pids: Vec::new(),
        replace_watch_pids: false,
        recovery: false,
        on_deliver,
        replace_on_deliver,
        on_deliver_wake_on_cc: false,
    };

    // A pull attach writes no intent and must earn no proof.
    assert!(matches!(
        scenario.daemon.request(register(None, false)).await,
        Response::Registered { .. }
    ));
    assert!(
        !scenario
            .daemon
            .intent_store()
            .load(&scenario.intent.id())
            .expect("reload")
            .is_armed(),
        "a pull register must never stamp an armed proof"
    );

    // Arming push does.
    assert!(matches!(
        scenario
            .daemon
            .request(register(Some(Vec::new()), true))
            .await,
        Response::Registered { .. }
    ));
    let armed = scenario
        .daemon
        .intent_store()
        .load(&scenario.intent.id())
        .expect("reload");
    assert!(
        armed.is_armed(),
        "the daemon must record that it armed push, so the record survives a crash before finalize"
    );
    assert_eq!(
        armed.state,
        IntentRecoveryState::Pending,
        "arming is not finalizing: the record is still never reconciled"
    );
    let report = scenario.daemon.reconcile_once().await;
    assert_eq!(
        report.restored, 0,
        "an armed pending record is still not claimable"
    );
    assert_eq!(report.inert, 1);
}

/// The deadlock the turn-boundary refresh exists to break, traced end to end at the durable level.
///
/// A bridge reload gives the producer a new pid and start time while the `live` record still names
/// the old pair. The daemon is then replaced. The successor has no member for the binding, and it
/// cannot make one: every pass fails `producer_identity_mismatch` against the stale identity. So a
/// refresh gated on `push_registered` was gated on the exact thing it existed to restore, and the
/// binding never recovered.
///
/// Being `live` is itself durable proof that the binding was armed, so the refresh is admitted with
/// no member at all — while an unarmed `pending` record in the same position is not, which is what
/// keeps a merely-existing bridge from arming an attach that was never registered.
#[tokio::test]
async fn station_intent_a_reloaded_producer_recovers_with_no_member_to_start_from() {
    let scenario = Scenario::new("intent-reload-deadlock", ProducerBehavior::Healthy).await;
    // The bridge reloaded: same endpoint, new process identity.
    let mut reloaded = scenario.intent.clone();
    let real_start_time = reloaded.producer.start_time;
    reloaded.producer.start_time = real_start_time.wrapping_add(1);
    reloaded.generation = 2;
    scenario.reseed(&reloaded);

    // The daemon replacement: no member exists, and no pass can create one.
    for _ in 0..2 {
        let report = scenario.daemon.reconcile_once().await;
        assert_eq!(report.restored, 0);
    }
    assert_eq!(
        scenario.failure_code().as_deref(),
        Some("producer_identity_mismatch")
    );
    assert!(
        !scenario.member_push_registered().await,
        "the precondition of the deadlock: there is no member, and none can be created"
    );

    // The turn-boundary hook's decision, with the daemon contributing nothing.
    let durable = scenario
        .daemon
        .intent_store()
        .load(&scenario.intent.id())
        .expect("reload");
    assert_eq!(
        telex::station_intent::finalize_admission(
            durable.state,
            durable.is_armed(),
            /* armed_now */ false,
        ),
        telex::station_intent::FinalizeAdmission::Refresh,
        "an already-live record must be refreshable without a live member"
    );
    // ...and the same decision for a record that was never armed is a refusal, which is the
    // security property this must not trade away.
    assert_eq!(
        telex::station_intent::finalize_admission(
            IntentRecoveryState::Pending,
            /* armed_durably */ false,
            /* armed_now */ false,
        ),
        telex::station_intent::FinalizeAdmission::RefusedNotArmed
    );

    // Exactly the durable write `finalize_intent` performs once the live bridge has been probed.
    scenario
        .daemon
        .intent_store()
        .update_locked(&scenario.intent.id(), |intent| {
            intent.producer.start_time = real_start_time;
            intent.state = IntentRecoveryState::Live;
            true
        })
        .expect("refresh")
        .expect("the record must still exist");

    let report = scenario.daemon.reconcile_once().await;
    assert_eq!(
        report.restored, 1,
        "with the identity re-recorded, the very next pass restores the binding: the failure \
         ladder was earned by a producer that no longer exists, and a durable state transition \
         drops it with the descriptor it described"
    );
    assert!(scenario.member_push_registered().await);
}

/// A bridge *reload* — `extensions_reload`, `/clear`, an extension-host restart — gives the
/// producer a new pid and start time while a `live` intent still names the old pair. Treating
/// that as terminal parked the binding for the quarantine hour with no automatic path back, on
/// the single most routine event in a Copilot session.
#[tokio::test]
async fn station_intent_a_reloaded_producer_is_retried_not_parked() {
    let scenario = Scenario::new("intent-reload-identity", ProducerBehavior::Healthy).await;
    let mut reloaded = scenario.intent.clone();
    reloaded.producer.start_time = reloaded.producer.start_time.wrapping_add(1);
    reloaded.generation = 2;
    scenario.reseed(&reloaded);

    let report = scenario.daemon.reconcile_once().await;
    assert_eq!(report.restored, 0);
    assert_eq!(
        report.failed, 1,
        "an identity mismatch is retryable: the turn-boundary hook refreshes the recorded \
         identity, and the ladder is what lets that heal"
    );
    assert_eq!(
        scenario.failure_code().as_deref(),
        Some("producer_identity_mismatch")
    );
    let entry = scenario
        .daemon
        .intent_index()
        .entries
        .values()
        .next()
        .cloned()
        .expect("an index entry");
    let next = entry.next_attempt_ms.expect("a scheduled next attempt");
    let delay = next - entry.last_attempt_ms.expect("a last attempt");
    assert!(
        delay < 60_000,
        "a reloaded producer must not be parked on the quarantine cadence (got {delay} ms)"
    );
}

// ---------------------------------------------------------------------------------------------
// M2: explicit withdrawal is one fallible, linearized operation
//
// Every test below is a path that previously tore down membership while leaving durable desired
// state saying "restore push" — so the next reconcile pass, or the next daemon, brought the
// station back.
// ---------------------------------------------------------------------------------------------

/// A reset with **no member at all** is the case the old member-derived withdrawal could not see:
/// with nothing in `affected` it withdrew nothing, and the manifest — the only remaining record of
/// the binding, and precisely what a pass restores from — survived untouched.
#[tokio::test]
async fn station_intent_reset_withdraws_a_binding_with_no_member() {
    let scenario = Scenario::new("intent-reset-memberless", ProducerBehavior::Healthy).await;
    scenario.daemon.reconcile_once().await;
    assert!(scenario.member_push_registered().await);

    // The shape a daemon restart leaves behind: durable manifest, no in-memory member.
    scenario.daemon.forget_member(
        &scenario.store_key,
        &scenario.intent.session_id,
        &scenario.intent.address,
    );
    assert!(!scenario.member_push_registered().await);

    let reset = scenario
        .daemon
        .request(Request::Reset {
            store_key: scenario.store_key.clone(),
            address: scenario.intent.address.clone(),
            proof: Some(scenario.daemon.admin_cap().to_string()),
        })
        .await;
    assert!(matches!(reset, Response::Ack { .. }), "{reset:?}");

    assert_eq!(
        scenario
            .daemon
            .intent_store()
            .load(&scenario.intent.id())
            .expect("the manifest is still readable")
            .state,
        IntentRecoveryState::Revoked,
        "a memberless reset must still withdraw the desired state"
    );
    let report = scenario.daemon.reconcile_once().await;
    assert_eq!(
        report.restored, 0,
        "and the next pass must not bring the station back"
    );
    assert!(!scenario.member_push_registered().await);
}

/// A reset of a station that is **already idle** withdrew nothing either: marking an idle member
/// idle changes no members, so the member-derived withdrawal set was empty.
///
/// The live manifest here models the state a re-attach (or a hand-edited/stale record) leaves: the
/// operator resets again precisely because the station is still armed in durable desired state.
#[tokio::test]
async fn station_intent_reset_withdraws_an_already_idle_binding() {
    let scenario = Scenario::new("intent-reset-idle", ProducerBehavior::Healthy).await;
    scenario.daemon.reconcile_once().await;
    assert!(scenario.member_push_registered().await);

    let first = scenario
        .daemon
        .request(Request::Reset {
            store_key: scenario.store_key.clone(),
            address: scenario.intent.address.clone(),
            proof: Some(scenario.daemon.admin_cap().to_string()),
        })
        .await;
    assert!(matches!(first, Response::Ack { .. }), "{first:?}");
    assert!(
        scenario
            .status()
            .await
            .members
            .iter()
            .any(|m| m.address == scenario.intent.address && m.idle),
        "precondition: the member is idle before the second reset"
    );

    let mut relive = scenario.intent.clone();
    relive.state = IntentRecoveryState::Live;
    relive.generation = 42;
    scenario.reseed(&relive);

    let second = scenario
        .daemon
        .request(Request::Reset {
            store_key: scenario.store_key.clone(),
            address: scenario.intent.address.clone(),
            proof: Some(scenario.daemon.admin_cap().to_string()),
        })
        .await;
    assert!(matches!(second, Response::Ack { .. }), "{second:?}");

    let stored = scenario
        .daemon
        .intent_store()
        .load(&scenario.intent.id())
        .expect("the manifest is still readable");
    assert_eq!(
        stored.state,
        IntentRecoveryState::Revoked,
        "an already-idle member must not make a reset a no-op for desired state"
    );
    assert!(stored.generation > 42, "the withdrawal moved the record");
    assert_eq!(scenario.daemon.reconcile_once().await.restored, 0);
}

/// Withdrawing an unfinalized `pending` record **deletes** it rather than leaving an identity-less
/// tombstone: its producer block is still the attach-time placeholder, so a `revoked` record here
/// would occupy the binding for the seven-day terminal TTL and hand every re-attach in that window
/// a finished lifecycle's clock.
#[tokio::test]
async fn station_intent_reset_deletes_an_unfinalized_pending_record() {
    let scenario = Scenario::new("intent-reset-pending", ProducerBehavior::Healthy).await;
    let mut pending = scenario.intent.clone();
    pending.state = IntentRecoveryState::Pending;
    pending.generation = 3;
    scenario.reseed(&pending);

    let reset = scenario
        .daemon
        .request(Request::Reset {
            store_key: scenario.store_key.clone(),
            address: scenario.intent.address.clone(),
            proof: Some(scenario.daemon.admin_cap().to_string()),
        })
        .await;
    assert!(matches!(reset, Response::Ack { .. }), "{reset:?}");
    assert!(
        scenario
            .daemon
            .intent_store()
            .load(&scenario.intent.id())
            .is_err(),
        "an unfinalized attach is deleted by an explicit withdrawal, never tombstoned"
    );
    assert!(
        scenario
            .daemon
            .intent_statuses()
            .iter()
            .all(|row| row.address != scenario.intent.address),
        "and the index must not keep projecting a row for a manifest that no longer exists"
    );
}

/// Session end takes the same route, and it enumerates the *scope* rather than the members it
/// changed — so an unfinalized attach for an ended session is withdrawn too.
#[tokio::test]
async fn station_intent_session_end_deletes_an_unfinalized_pending_record() {
    let scenario = Scenario::new("intent-end-pending", ProducerBehavior::Healthy).await;
    let mut pending = scenario.intent.clone();
    pending.state = IntentRecoveryState::Pending;
    pending.generation = 5;
    scenario.reseed(&pending);

    let ended = scenario
        .daemon
        .request(Request::SessionEnd {
            store_key: scenario.store_key.clone(),
            session_id: scenario.intent.session_id.clone(),
            proof: Some(scenario.daemon.admin_cap().to_string()),
        })
        .await;
    assert!(matches!(ended, Response::Ack { .. }), "{ended:?}");
    assert!(
        scenario
            .daemon
            .intent_store()
            .load(&scenario.intent.id())
            .is_err(),
        "an ended session leaves no claimable pending record behind"
    );
    assert_eq!(scenario.daemon.reconcile_once().await.restored, 0);
}

/// **Withdrawal beats a restoration already in flight.**
///
/// The restore chain is a long sequence of awaits, and only a *detach* leaves a durable tombstone
/// the chain re-checks. A reset or a session end leaves none, so a pass that had already loaded the
/// manifest went on to publish an armed push member the desired state no longer authorized — and
/// only a later pass could notice.
///
/// This drives the guard-free inner routine with a manifest captured *before* the withdrawal, which
/// is exactly the stale snapshot an in-flight pass holds.
#[tokio::test]
async fn station_intent_a_concurrent_withdrawal_beats_a_reconcile_in_flight() {
    let scenario = Scenario::new("intent-withdraw-race", ProducerBehavior::Healthy).await;
    let in_flight = scenario.intent.clone();

    let reset = scenario
        .daemon
        .request(Request::Reset {
            store_key: scenario.store_key.clone(),
            address: scenario.intent.address.clone(),
            proof: Some(scenario.daemon.admin_cap().to_string()),
        })
        .await;
    assert!(matches!(reset, Response::Ack { .. }), "{reset:?}");

    let outcome = scenario
        .daemon
        .reconcile_intent_under_admission_guard(&in_flight)
        .await;
    assert!(
        outcome.contains("Revoked") && outcome.contains("withdrawn"),
        "the member commit must be refused by the manifest re-check, not by luck: got {outcome}"
    );
    assert!(
        !scenario.member_push_registered().await,
        "the withdrawal must win over the restoration it raced"
    );
}

/// The same race with a **deleted** record: a withdrawal of a `pending` binding leaves nothing at
/// all, and an absent manifest is never read as consent to publish.
#[tokio::test]
async fn station_intent_a_reconcile_cannot_publish_from_a_record_that_was_deleted() {
    let scenario = Scenario::new("intent-withdraw-deleted", ProducerBehavior::Healthy).await;
    let in_flight = scenario.intent.clone();

    scenario
        .daemon
        .intent_store()
        .withdraw_binding(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address,
            9_000,
        )
        .expect("withdraw");
    // Re-seed as pending and withdraw again, so the record is genuinely gone rather than revoked.
    let mut pending = scenario.intent.clone();
    pending.state = IntentRecoveryState::Pending;
    scenario.reseed(&pending);
    scenario
        .daemon
        .intent_store()
        .withdraw_binding(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address,
            9_500,
        )
        .expect("withdraw pending");
    assert!(scenario
        .daemon
        .intent_store()
        .load(&scenario.intent.id())
        .is_err());

    let outcome = scenario
        .daemon
        .reconcile_intent_under_admission_guard(&in_flight)
        .await;
    assert!(
        outcome.contains("Revoked") && outcome.contains("withdrawn"),
        "an absent manifest is never read as consent to publish, got {outcome}"
    );
    assert!(!scenario.member_push_registered().await);
}

/// A **stale reconcile outcome** must not delete a record it knows nothing about.
///
/// The pass decides "tombstoned" against the generation it loaded; a re-attach can write a fresh
/// `pending` record before the outcome is applied. Withdrawing unconditionally there would destroy
/// an attach in progress — and, because `pending` withdrawal deletes, destroy it irrecoverably.
#[tokio::test]
async fn station_intent_a_stale_pass_cannot_delete_a_fresh_re_attach() {
    let scenario = Scenario::new("intent-stale-vs-reattach", ProducerBehavior::Healthy).await;
    let store = scenario.daemon.intent_store();

    // A record the pass decided against, at a generation a re-attach then moves past.
    let observed = store.load(&scenario.intent.id()).expect("load").generation;
    let mut reattached = scenario.intent.clone();
    reattached.state = IntentRecoveryState::Pending;
    reattached.generation = observed + 7;
    scenario.reseed(&reattached);

    let superseded = store
        .withdraw_binding_at_generation(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address,
            Some(observed),
            10_000,
        )
        .expect("stale withdrawal");
    assert_eq!(
        superseded,
        telex::station_intent::Withdrawal::Superseded {
            generation: observed + 7
        }
    );
    assert_eq!(
        store
            .load(&scenario.intent.id())
            .expect("the fresh attach survives")
            .state,
        IntentRecoveryState::Pending,
        "a stale pass must never delete the record a re-attach just wrote"
    );
}

/// A withdrawal that cannot be *decided* fails the teardown rather than reporting success.
///
/// This is the whole point of making withdrawal fallible. An unsupported schema version is what a
/// rollback leaves behind: it is never deleted and never clobbered, so the honest answer is "the
/// desired state for this binding was not withdrawn" — and the operator has to see that, because a
/// newer build reading that record will still restore push from it.
#[tokio::test]
async fn station_intent_a_reset_that_cannot_withdraw_reports_failure() {
    let scenario = Scenario::new("intent-reset-unreadable", ProducerBehavior::Healthy).await;
    let path = scenario
        .daemon
        .intent_store()
        .path_for(&scenario.intent.id());
    let raw = std::fs::read_to_string(&path).expect("read intent");
    let mut document: serde_json::Value = serde_json::from_str(&raw).expect("parse intent");
    document["schema_version"] = serde_json::json!(99);
    let _ = std::fs::remove_file(&path);
    telex::platform_fs::write_owner_only_file_atomic(
        &path,
        serde_json::to_vec(&document).expect("encode").as_slice(),
    )
    .expect("write skewed intent");

    let reset = scenario
        .daemon
        .request(Request::Reset {
            store_key: scenario.store_key.clone(),
            address: scenario.intent.address.clone(),
            proof: Some(scenario.daemon.admin_cap().to_string()),
        })
        .await;
    match reset {
        Response::Error { .. } => {}
        other => panic!("a reset that could not withdraw must not report success: {other:?}"),
    }
    assert!(
        path.exists(),
        "and the manifest a rollback left behind is still there, untouched"
    );
}

/// Withdrawal is idempotent across the paths that legitimately overlap: a detach, a reset, and a
/// session end can all name one binding, and the second and third are successes.
#[tokio::test]
async fn station_intent_overlapping_teardowns_are_idempotent() {
    let scenario = Scenario::new("intent-idempotent-teardown", ProducerBehavior::Healthy).await;
    scenario.daemon.reconcile_once().await;

    for request in [
        Request::Detach {
            store_key: scenario.store_key.clone(),
            session_id: scenario.intent.session_id.clone(),
            address: scenario.intent.address.clone(),
        },
        Request::Reset {
            store_key: scenario.store_key.clone(),
            address: scenario.intent.address.clone(),
            proof: Some(scenario.daemon.admin_cap().to_string()),
        },
        Request::SessionEnd {
            store_key: scenario.store_key.clone(),
            session_id: scenario.intent.session_id.clone(),
            proof: Some(scenario.daemon.admin_cap().to_string()),
        },
    ] {
        let response = scenario.daemon.request(request).await;
        assert!(
            matches!(response, Response::Ack { .. }),
            "overlapping teardowns must all succeed: {response:?}"
        );
    }
    assert_eq!(
        scenario
            .daemon
            .intent_store()
            .load(&scenario.intent.id())
            .expect("the tombstone is still there")
            .state,
        IntentRecoveryState::Revoked
    );
    assert_eq!(scenario.daemon.reconcile_once().await.restored, 0);
}

// ---------------------------------------------------------------------------------------------
// The published 4-second pass/admin bound
// ---------------------------------------------------------------------------------------------
//
// The bound is one absolute number for the whole pass *and* for the admin request that awaits it.
// What made it untrue was never the arithmetic in the ordinary case; it was the places where a
// phase started a fresh clock of its own — a post-wave withdrawal waiting the full per-intent
// admission timeout, an admin backstop set outside the pass deadline, and blocking filesystem
// phases whose cooperative checks cannot bound the call that is currently blocked.

/// The admin request answers inside the published bound even while the binding's admission guard
/// is held by someone else — the case where the pass legitimately spends its whole wave budget.
#[tokio::test]
async fn station_intent_the_admin_request_answers_inside_the_published_pass_bound() {
    use telex::daemon_reconcile::{RECONCILE_ADMIN_DEADLINE, RECONCILE_PASS_DEADLINE};

    let scenario = Scenario::new("intent-admin-bound", ProducerBehavior::Healthy).await;
    // A concurrent register/detach holds this guard across backend work, so the wave below waits
    // on it and burns its per-intent timeout. That is the pass this bound has to survive.
    let _held = scenario
        .daemon
        .hold_delivery_admission(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address,
        )
        .await;

    let started = std::time::Instant::now();
    let response = scenario
        .daemon
        .request(Request::ReconcileIntents {
            proof: Some(scenario.daemon.admin_cap().to_string()),
            scope: Some(scenario.store_key.clone()),
        })
        .await;
    let elapsed = started.elapsed();

    assert!(
        matches!(response, Response::Reconciled { .. }),
        "the admin surface must answer with a report, not hang: {response:?}"
    );
    assert_eq!(
        RECONCILE_ADMIN_DEADLINE, RECONCILE_PASS_DEADLINE,
        "the admin backstop is the pass bound made observable at the IPC surface, not a second, \
         looser bound"
    );
    assert!(
        elapsed <= RECONCILE_PASS_DEADLINE + Duration::from_millis(750),
        "a contended pass answered in {elapsed:?}, past the published {RECONCILE_PASS_DEADLINE:?} \
         bound (allowing for test-runner scheduling)"
    );
}

/// A terminal outcome's withdrawal must be bounded by what is left of the pass, not by a fresh
/// per-intent admission timeout.
///
/// Outcome application runs after a wave that may already have spent the whole budget. Starting a
/// new three-second admission wait there is how a pass bounded at four seconds answered its admin
/// caller at seven — and it is invisible in the ordinary case, because an uncontended guard is
/// taken instantly.
#[tokio::test]
async fn station_intent_a_terminal_withdrawal_is_bounded_by_the_remaining_pass_deadline() {
    use telex::daemon_reconcile::RECONCILE_PER_INTENT_TIMEOUT;

    let scenario = Scenario::new("intent-withdraw-bound", ProducerBehavior::Healthy).await;
    let _held = scenario
        .daemon
        .hold_delivery_admission(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address,
        )
        .await;

    // What `apply_outcome` passes when a wave has left the pass a few hundred milliseconds.
    let remaining = Duration::from_millis(300);
    let started = std::time::Instant::now();
    let outcome = scenario
        .daemon
        .withdraw_within(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address,
            remaining,
        )
        .await;
    let elapsed = started.elapsed();

    assert!(
        outcome.is_err(),
        "a withdrawal that could not take the guard must say so rather than report a teardown \
         that did not happen: {outcome:?}"
    );
    assert!(
        elapsed < RECONCILE_PER_INTENT_TIMEOUT,
        "the withdrawal waited {elapsed:?}, which is the fresh per-intent budget rather than what \
         the pass had left ({remaining:?})"
    );
    assert!(
        elapsed >= remaining,
        "and it must actually use the budget it was given, not give up immediately: {elapsed:?}"
    );
    // The record is untouched, so the next pass re-derives the same decision.
    assert!(scenario
        .daemon
        .intent_store()
        .load(&scenario.intent.id())
        .is_ok());
}

/// The whole pass, not just the request, stays inside the bound under the same contention — and
/// still publishes an honest report.
#[tokio::test]
async fn station_intent_a_contended_pass_stays_inside_its_deadline() {
    use telex::daemon_reconcile::RECONCILE_PASS_DEADLINE;

    let scenario = Scenario::new("intent-pass-bound", ProducerBehavior::Healthy).await;
    let _held = scenario
        .daemon
        .hold_delivery_admission(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address,
        )
        .await;

    let started = std::time::Instant::now();
    let report = scenario.daemon.reconcile_once().await;
    let elapsed = started.elapsed();

    assert!(report.ran, "the pass ran; it was merely contended");
    assert!(
        elapsed <= RECONCILE_PASS_DEADLINE + Duration::from_millis(750),
        "a contended pass took {elapsed:?}, past its own {RECONCILE_PASS_DEADLINE:?} deadline"
    );
    assert!(
        report.duration_ms <= RECONCILE_PASS_DEADLINE.as_millis() as u64 + 750,
        "the pass must also *report* a duration inside its bound: {}ms",
        report.duration_ms
    );
    assert_eq!(
        report.restored, 0,
        "a guard this pass could not take is never a restoration"
    );
}

// ---------------------------------------------------------------------------------------------
// Deadline edge: what a pass is allowed to do after it has answered
// ---------------------------------------------------------------------------------------------

/// A pass handed a deadline that has already elapsed must publish **nothing** — and must still be
/// publishing nothing once the runtime has had time to drain whatever it might have started.
///
/// This is the deterministic half of the deadline-edge property, and it is deterministic precisely
/// because the deadline is an input rather than something a test has to race a wall clock into.
/// Every phase is on the far side of its budget from the first instruction, so the pass has to take
/// the "not enough reserve to start" branch everywhere at once: no cursor advance, no evidence
/// rewrite, no event-log append, no member.
///
/// Before the correction each of those was a `spawn_blocking` whose *wait* was bounded and whose
/// *work* was not, so a pass at its deadline still launched three filesystem writes and returned.
/// They landed whenever the filesystem got to them, which for a request-originated pass is after the
/// caller has already been answered — the caller could watch its own cursor move after being told
/// the pass was truncated. The settle window below is what turns "did not publish yet" into "will
/// not publish".
#[tokio::test]
async fn station_intent_a_pass_past_its_deadline_publishes_nothing_before_or_after_returning() {
    let scenario = Scenario::new("intent-deadline-edge", ProducerBehavior::Healthy).await;
    let cursor_before = scenario.daemon.scan_cursor_bytes();
    let log_before = scenario.daemon.reconcile_event_log_bytes();
    let members_before = scenario.daemon.member_keys();

    let report = scenario
        .daemon
        .reconcile_once_until(std::time::Instant::now())
        .await;

    assert!(
        report.deadline_reached || !report.ran,
        "a pass with no budget must say so rather than report a clean pass: {report:?}"
    );
    assert_eq!(
        report.restored, 0,
        "a pass with no budget cannot have restored anything"
    );

    // Nothing published by the time it returned...
    assert_eq!(scenario.daemon.member_keys(), members_before);
    assert_eq!(scenario.daemon.scan_cursor_bytes(), cursor_before);
    assert_eq!(scenario.daemon.reconcile_event_log_bytes(), log_before);

    // ...and nothing published afterwards either. A write that was merely *unwaited-for* rather
    // than *unstarted* would land in this window.
    tokio::time::sleep(POST_RESPONSE_SETTLE).await;
    assert_eq!(
        scenario.daemon.member_keys(),
        members_before,
        "a member appeared after the pass had already returned"
    );
    assert_eq!(
        scenario.daemon.scan_cursor_bytes(),
        cursor_before,
        "the round-robin cursor moved after the pass had already returned"
    );
    assert_eq!(
        scenario.daemon.reconcile_event_log_bytes(),
        log_before,
        "an evidence-log append landed after the pass had already returned"
    );
}

/// How long to wait before re-checking that nothing further was published.
///
/// Sized against the two things that could still be in flight — an abandoned `spawn_blocking` write
/// and a wave task the pass no longer joins — so it is comfortably longer than either would take on
/// a healthy filesystem. It cannot make the test flaky in the passing direction: a longer wait can
/// only ever find *more* evidence of a late publication.
const POST_RESPONSE_SETTLE: Duration = Duration::from_millis(750);

/// The same response-bound property across the **real IPC surface**, at the deadline edge.
///
/// The contention is the point. Holding the binding's admission guard makes the wave spend its whole
/// `RECONCILE_PER_INTENT_TIMEOUT`, which is the case that used to expose two clocks pretending to be
/// one bound: the handler spawned the pass and raced it with a `timeout` of the same length but a
/// *different origin*, so it could answer `admin_deadline` while the pass was still mid-wave and the
/// pass would then go on registering members, advancing cursors, and publishing a report belonging
/// to a request that had already been answered. One request-originated deadline prevents those
/// asynchronous pass publications. It does not claim that an already-entered blocking filesystem
/// rename or unlink cannot finish later; those mutations are guarded by generation and OS lock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn station_intent_the_ipc_reconcile_round_trip_has_no_late_pass_publication() {
    use telex::daemon_reconcile::{RECONCILE_MAX_CONCURRENCY, RECONCILE_PASS_DEADLINE};

    let scenario = Scenario::new("intent-ipc-edge", ProducerBehavior::Healthy).await;
    // A whole wave's worth of bindings, every one of them contended, plus one more that a second
    // wave would have to pick up. The extra binding is the canary: it is exactly what a pass that
    // kept running past the response would publish next.
    let mut held = vec![
        scenario
            .daemon
            .hold_delivery_admission(
                &scenario.store_key,
                &scenario.intent.session_id,
                &scenario.intent.address,
            )
            .await,
    ];
    for index in 1..=RECONCILE_MAX_CONCURRENCY {
        let mut intent = scenario.intent.clone();
        intent.address = format!("addr:edge-{index}");
        scenario.reseed(&intent);
        if index < RECONCILE_MAX_CONCURRENCY {
            held.push(
                scenario
                    .daemon
                    .hold_delivery_admission(
                        &scenario.store_key,
                        &intent.session_id,
                        &intent.address,
                    )
                    .await,
            );
        }
    }

    let _server = scenario.daemon.serve_ipc();
    let mut client = scenario.daemon.connect_ipc(&scenario.store_key).await;

    let started = std::time::Instant::now();
    let response = client
        .request(&Request::ReconcileIntents {
            proof: Some(scenario.daemon.admin_cap().to_string()),
            scope: Some(scenario.store_key.clone()),
        })
        .await
        .expect("the reconcile request completed over the real IPC surface");
    let round_trip = started.elapsed();

    let Response::Reconciled { report } = response else {
        panic!("the admin surface must answer with a report: {response:?}");
    };
    assert!(
        report.ran,
        "the caller must receive the pass's own report, not a placeholder standing in for a pass \
         still running behind its back: {report:?}"
    );
    assert!(
        round_trip <= RECONCILE_PASS_DEADLINE,
        "a real IPC reconcile round trip took {round_trip:?}, past the published \
         {RECONCILE_PASS_DEADLINE:?} bound"
    );

    let members_after = scenario.daemon.member_keys();
    let cursor_after = scenario.daemon.scan_cursor_bytes();
    let reports_after = scenario.daemon.reconcile_reports().borrow().pass_seq;

    tokio::time::sleep(POST_RESPONSE_SETTLE).await;

    assert_eq!(
        scenario.daemon.member_keys(),
        members_after,
        "a member was published after the reconcile request had already been answered"
    );
    assert_eq!(
        scenario.daemon.scan_cursor_bytes(),
        cursor_after,
        "the round-robin cursor moved after the reconcile request had already been answered"
    );
    assert_eq!(
        scenario.daemon.reconcile_reports().borrow().pass_seq,
        reports_after,
        "a pass report was published after the reconcile request had already been answered"
    );
    drop(held);
}

// ---------------------------------------------------------------------------------------------
// Reverse-order teardown races: the reconciler wins the admission race, the teardown arrives second
// ---------------------------------------------------------------------------------------------
//
// Every test in this section runs the interleaving that actually breaks. Before the correction,
// reset, detach, session end, and the fallback downgrade all read membership and mutated lifecycle
// state *outside* the per-binding delivery-admission guard and took it only for the durable
// withdrawal. That is not a linearization: a pass holding admission could publish an armed push
// member in the gap, and the teardown would then revoke the manifest while leaving behind exactly
// the member the manifest had authorized -- live push coverage with no durable record saying it
// should exist, and nothing left to withdraw it by.
//
// The publication is modelled rather than raced against a real pass, deliberately. A real pass
// would have to be timed, and a timed test either flakes or stops testing the interleaving. Here
// the guard *is* the synchronization, so the order is fixed by construction: the teardown cannot
// proceed until the test drops the guard, and the member is published before it does.

/// How long a teardown is given to prove it is *blocked* on admission.
///
/// Only a lower bound on "did not proceed", so it cannot become flaky by being slow. It stays well
/// under the teardown deadline, or the operation would legitimately give up before the guard is
/// released.
const BLOCKED_OBSERVATION: Duration = Duration::from_millis(150);

impl Scenario {
    /// The durable desired state must be gone: either the record was deleted (a `pending` record is
    /// withdrawn by deletion) or it is an explicit `Revoked` tombstone. Anything else is a record a
    /// later pass is entitled to restore push from.
    fn assert_intent_withdrawn(&self, what: &str) {
        if let Ok(record) = self.daemon.intent_store().load(&self.intent.id()) {
            assert_eq!(
                record.state,
                IntentRecoveryState::Revoked,
                "{what} left durable desired state at {:?}, which the next pass restores from",
                record.state
            );
        }
    }

    /// Nothing is left for a pass to restore. The end-to-end statement of the invariant: no armed
    /// member, and no record that would produce one.
    async fn assert_nothing_restorable(&self, what: &str) {
        assert_eq!(
            self.daemon.reconcile_once().await.restored,
            0,
            "{what} must leave nothing for the next pass to restore"
        );
        assert!(
            !self.daemon.has_active_push_member(
                &self.store_key,
                &self.intent.session_id,
                &self.intent.address
            ),
            "{what} left an active push member behind after a full pass"
        );
    }

    /// Model the reconciler finishing a restore it was already admitted for.
    async fn reconciler_publishes_push_member(&self) {
        self.daemon
            .publish_push_member_unadmitted(
                &self.store_key,
                &self.intent.session_id,
                &self.intent.address,
            )
            .await;
    }
}

/// `Reset` arriving second: the reconciler already holds admission and publishes an armed member
/// while the reset waits for it.
///
/// The pre-correction reset marked members idle first and took the guard only to withdraw, so this
/// member -- published after that marking -- stayed active and armed while the manifest that
/// authorized it was revoked out from under it.
#[tokio::test]
async fn station_intent_a_reset_that_loses_the_admission_race_leaves_no_push_member() {
    let scenario = Scenario::new("intent-reset-race", ProducerBehavior::Healthy).await;
    let held = scenario
        .daemon
        .hold_delivery_admission(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address,
        )
        .await;

    let handle = scenario.daemon.handle();
    let request = Request::Reset {
        store_key: scenario.store_key.clone(),
        address: scenario.intent.address.clone(),
        proof: Some(scenario.daemon.admin_cap().to_string()),
    };
    let reset = tokio::spawn(async move { handle.request(request).await });

    tokio::time::sleep(BLOCKED_OBSERVATION).await;
    assert!(
        !reset.is_finished(),
        "the reset answered while the binding's admission guard was held by someone else; it \
         cannot have serialized against the reconciler"
    );

    scenario.reconciler_publishes_push_member().await;
    drop(held);

    let response = reset.await.expect("reset task");
    assert!(
        matches!(response, Response::Ack { .. }),
        "the reset must complete once it is admitted: {response:?}"
    );
    assert!(
        !scenario.daemon.has_active_push_member(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address
        ),
        "the reset revoked the manifest but left the member the reconciler published still armed"
    );
    scenario.assert_intent_withdrawn("reset");
    scenario.assert_nothing_restorable("reset").await;
}

/// `Detach` arriving second. Detach *removes* the member rather than idling it, so the assertion is
/// stronger: no member record at all may survive.
///
/// Detach also has an ordering constraint the correction must not break -- the durable tombstone and
/// lease release happen first, the local withdrawal second -- so this is equally the proof that
/// holding admission across that backend work neither deadlocks nor reorders it.
#[tokio::test]
async fn station_intent_a_detach_that_loses_the_admission_race_removes_the_published_member() {
    let scenario = Scenario::new("intent-detach-race", ProducerBehavior::Healthy).await;
    let held = scenario
        .daemon
        .hold_delivery_admission(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address,
        )
        .await;

    let handle = scenario.daemon.handle();
    let request = Request::Detach {
        store_key: scenario.store_key.clone(),
        session_id: scenario.intent.session_id.clone(),
        address: scenario.intent.address.clone(),
    };
    let detach = tokio::spawn(async move { handle.request(request).await });

    tokio::time::sleep(BLOCKED_OBSERVATION).await;
    assert!(
        !detach.is_finished(),
        "the detach answered while the binding's admission guard was held; it read membership \
         outside the guard"
    );

    scenario.reconciler_publishes_push_member().await;
    drop(held);

    let response = detach.await.expect("detach task");
    assert!(
        matches!(response, Response::Ack { .. }),
        "the detach must complete once it is admitted: {response:?}"
    );
    assert!(
        !scenario.daemon.has_member(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address
        ),
        "the detach removed the member it saw before admission and left the one published inside it"
    );
    scenario.assert_intent_withdrawn("detach");
    scenario.assert_nothing_restorable("detach").await;
}

/// `SessionEnd` arriving second. An ended session must not be attended by anything, including a
/// member published by a pass that was mid-restore when the session ended.
#[tokio::test]
async fn station_intent_a_session_end_that_loses_the_admission_race_leaves_no_push_member() {
    let scenario = Scenario::new("intent-session-end-race", ProducerBehavior::Healthy).await;
    let held = scenario
        .daemon
        .hold_delivery_admission(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address,
        )
        .await;

    let handle = scenario.daemon.handle();
    let request = Request::SessionEnd {
        store_key: scenario.store_key.clone(),
        session_id: scenario.intent.session_id.clone(),
        proof: Some(scenario.daemon.admin_cap().to_string()),
    };
    let session_end = tokio::spawn(async move { handle.request(request).await });

    tokio::time::sleep(BLOCKED_OBSERVATION).await;
    assert!(
        !session_end.is_finished(),
        "the session end answered while the binding's admission guard was held; its lifecycle \
         mutation is not serialized against the reconciler"
    );

    scenario.reconciler_publishes_push_member().await;
    drop(held);

    let response = session_end.await.expect("session-end task");
    assert!(
        matches!(response, Response::Ack { .. }),
        "the session end must complete once it is admitted: {response:?}"
    );
    assert!(
        !scenario.daemon.has_active_push_member(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address
        ),
        "the ended session is still attended by an armed push member"
    );
    scenario.assert_intent_withdrawn("session end");
    scenario.assert_nothing_restorable("session end").await;
}

/// The fallback push-to-pull downgrade arriving second.
///
/// The downgrade is `Register { on_deliver: None, replace_on_deliver: true }`, and the daemon now
/// performs the intent withdrawal *inside* that transition. The CLI used to do it afterwards, from
/// its own process, with the daemon's admission guard released in between -- which is this exact
/// race, except that the restored push member then sat alongside the pull waiter the downgrade had
/// just installed, delivering everything twice.
#[tokio::test]
async fn station_intent_a_fallback_downgrade_that_loses_the_admission_race_clears_push() {
    let scenario = Scenario::new("intent-fallback-race", ProducerBehavior::Healthy).await;
    let held = scenario
        .daemon
        .hold_delivery_admission(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address,
        )
        .await;

    let handle = scenario.daemon.handle();
    let request = fallback_downgrade_request(&scenario);
    let downgrade = tokio::spawn(async move { handle.request(request).await });

    tokio::time::sleep(BLOCKED_OBSERVATION).await;
    assert!(
        !downgrade.is_finished(),
        "the downgrade answered while the binding's admission guard was held"
    );

    scenario.reconciler_publishes_push_member().await;
    drop(held);

    let response = downgrade.await.expect("downgrade task");
    assert!(
        matches!(response, Response::Registered { .. }),
        "the downgrade must complete once it is admitted: {response:?}"
    );
    assert!(
        !scenario.daemon.has_active_push_member(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address
        ),
        "the downgrade installed a pull member but left push armed on the same binding"
    );
    scenario.assert_intent_withdrawn("fallback downgrade");
    scenario
        .assert_nothing_restorable("fallback downgrade")
        .await;
}

/// A downgrade whose intent cannot be withdrawn is refused rather than reporting a pull-only
/// registration whose desired state still says "restore push".
///
/// The failure half of the combined transition. Without it the fallback would report success, tear
/// down its bridge, and then be re-armed by the next pass from a record nobody could withdraw.
#[tokio::test]
async fn station_intent_a_fallback_downgrade_that_cannot_withdraw_is_refused() {
    let scenario = Scenario::new("intent-fallback-refuse", ProducerBehavior::Healthy).await;
    // The rollback shape: a record no build in this process can decide about, so it is never
    // deleted and never clobbered.
    let path = scenario
        .daemon
        .intent_store()
        .path_for(&scenario.intent.id());
    let raw = std::fs::read_to_string(&path).expect("read intent");
    let mut document: serde_json::Value = serde_json::from_str(&raw).expect("parse intent");
    document["schema_version"] = serde_json::json!(99);
    let _ = std::fs::remove_file(&path);
    telex::platform_fs::write_owner_only_file_atomic(
        &path,
        serde_json::to_vec(&document).expect("encode").as_slice(),
    )
    .expect("write skewed intent");

    let response = scenario
        .daemon
        .request(fallback_downgrade_request(&scenario))
        .await;
    match response {
        Response::Error { .. } => {}
        other => panic!("a downgrade that could not withdraw must not report success: {other:?}"),
    }
    assert!(
        path.exists(),
        "and the record it could not decide about is untouched"
    );
    assert!(
        !scenario.daemon.has_member(
            &scenario.store_key,
            &scenario.intent.session_id,
            &scenario.intent.address
        ),
        "a refused downgrade must not publish the pull-only member either"
    );
}

/// Exactly what `telex copilot fallback` sends: the explicit push-to-pull downgrade, which is the
/// only request shape that clears an installed `on_deliver` and is therefore the one that owns the
/// matching intent withdrawal.
fn fallback_downgrade_request(scenario: &Scenario) -> Request {
    Request::Register {
        store_key: scenario.store_key.clone(),
        address: scenario.intent.address.clone(),
        session_id: scenario.intent.session_id.clone(),
        occupant: "fallback".to_string(),
        description: None,
        scope: None,
        tags: None,
        watch_pids: Vec::new(),
        replace_watch_pids: true,
        recovery: false,
        on_deliver: None,
        replace_on_deliver: true,
        on_deliver_wake_on_cc: false,
    }
}
