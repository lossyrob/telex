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
    // `attempted` sums *attempts*, not distinct intents: a regression where the cursor stalls and
    // the same 64 intents are re-attempted every pass satisfies the sum while the tail starves.
    // The index is keyed per binding, so counting entries that were actually attempted is the
    // assertion the row is really about.
    let distinct_attempted = daemon
        .intent_index()
        .entries
        .values()
        .filter(|entry| entry.attempts > 0)
        .count();
    assert_eq!(
        distinct_attempted, total,
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

/// The daemon writes the durable **armed proof** itself, at `Register`, so a crash anywhere between
/// arming push and the producer-side finalize leaves a record that says push was armed.
#[tokio::test]
async fn station_intent_the_daemon_stamps_the_armed_proof_when_it_arms_push() {
    let scenario = Scenario::new("intent-armed-proof", ProducerBehavior::Healthy).await;
    let mut pending = scenario.intent.clone();
    pending.state = IntentRecoveryState::Pending;
    pending.generation = 2;
    scenario.reseed(&pending);

    let register = |on_deliver: Option<Vec<String>>| Request::Register {
        store_key: scenario.store_key.clone(),
        address: scenario.intent.address.clone(),
        session_id: scenario.intent.session_id.clone(),
        occupant: "occupant".to_string(),
        description: None,
        scope: None,
        tags: None,
        watch_pids: Vec::new(),
        recovery: false,
        on_deliver,
        replace_on_deliver: true,
        on_deliver_wake_on_cc: false,
    };

    // A pull attach writes no intent and must earn no proof.
    assert!(matches!(
        scenario.daemon.request(register(None)).await,
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
        scenario.daemon.request(register(Some(Vec::new()))).await,
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
