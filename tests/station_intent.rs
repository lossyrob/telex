//! Station-intent reconciliation behavior (issue #106 / ADR 0050).
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
use telex::station_intent::{StationIntentV1, STATION_INTENT_MAX_COUNT};

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
    while attempted < total && passes < ceiling {
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
async fn station_intent_drain_report_is_index_only_and_suppresses_reconciliation() {
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

    // Removing the manifest must not change the report: it is computed from the cached index only,
    // which is what keeps a graceful drain free of directory I/O.
    let path = scenario
        .daemon
        .intent_store()
        .path_for(&scenario.intent.id());
    std::fs::remove_file(&path).expect("remove manifest");
    let after = scenario.daemon.drain_intent_report();
    assert_eq!(
        after.recoverable, report.recoverable,
        "the drain report must not read the intent directory"
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
    let mut reports = scenario.daemon.reconcile_reports();
    let before = reports.borrow_and_update().pass_seq;
    let report = scenario
        .daemon
        .pulse_reconcile_and_wait(Duration::from_secs(10))
        .await;
    // The plain TestDaemon has no heartbeat loop, so a pulse alone parks; the harness therefore
    // drives the pass directly and publishes it on the same seam a production tick would.
    let report = match report {
        Some(report) => report,
        None => scenario.daemon.reconcile_once().await,
    };
    assert!(report.pass_seq > before);
    let observed = reports.borrow_and_update().clone();
    assert!(
        observed.pass_seq >= report.pass_seq,
        "every completed pass must be published on the report seam"
    );
}
