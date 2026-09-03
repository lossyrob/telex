#![cfg(any(feature = "sqlite", feature = "postgres"))]
//! One backend-neutral Application Client conformance battery.
//!
//! The *same* semantic scenario functions run against an isolated SQLite store
//! and a credentialed Postgres schema. Every assertion is on public
//! `telex::application_client` types and results (plus the stable
//! `telex::model` rows those results embed). Private daemon/backend/install
//! helpers appear only as isolated setup, fault induction, or state induction,
//! never as an assertion surface -- each such use is called out inline.
//!
//! Isolation: `tests/support/telex_isolation.rs` builds a unique temp root with
//! its own `TELEX_HOME`, `TELEX_RUN_DIR`, `TELEX_DB`, `TELEX_CONFIG`,
//! `TELEX_INSTALL_ROOT` and lock-state dir, installs the *branch* binary
//! (absolute `CARGO_BIN_EXE_telex`) into a strict versioned layout, and points
//! `ApplicationDaemonBootstrap::InstalledCurrent` at it. Installed/user state
//! is never targeted. The Postgres leg additionally uses a per-run,
//! per-scenario schema and fails closed under `TELEX_PG_REQUIRE=1`.
//!
//! Coverage map: [`COVERAGE`] binds every scenario to the issue #152 families
//! (a..j) it proves, and [`assert_coverage`] fails if a scenario runs without a
//! mapping, a mapping never runs, or a family is unclaimed.

#[path = "support/telex_isolation.rs"]
mod isolation;

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use telex::application_client::{
    AckResult, AddressLifecycleResult, AddressSpec, ApplicationCapability, ApplicationClient,
    ApplicationClientConfig, ApplicationClientError, ApplicationDaemonBootstrap,
    ApplicationResponsibility, ApplicationStoreMaintenance, CompensationAction,
    CompoundDispositionRequest, CompoundStep, EvidenceState, LifecycleOperationKind,
    LogicalStoreId, MembershipLossReason, OperationId, OperationReconciliation,
    PrincipalVerification, RecordedOperationOutcome, RecoveryHandle, RecoveryPolicy, ReplyRequest,
    SendRequest, SourceReference, SourceResolution,
};
use telex::backend::Backend;
use telex::model::{
    now_ms, CompoundStepState, NewApplicationOperation, RetentionPolicy, StoreDeltaRetentionPolicy,
};
use telex::profiles::{BackendProfile, ConfigFile};

use isolation::{Isolation, ENV_LOCK};

// ----------------------------------------------------------------------------------------
// Coverage map: scenario -> issue #152 family letters
// ----------------------------------------------------------------------------------------

/// Every scenario in the shared battery and the families it proves.
///
/// a identity | b typed loss/recovery/collision | c multi-address lifecycle |
/// d send-only vs bidirectional | e receipt axes | f operations/recovery |
/// g history/source | h deltas | i compound | j schema/cleanup/provenance.
const COVERAGE: &[(&str, &str)] = &[
    ("identity_is_fresh_stable_and_presentation_independent", "a"),
    (
        "strict_recovery_refuses_repair_and_bounded_repair_reattaches",
        "b",
    ),
    ("restart_loses_membership_and_reports_typed_loss", "b"),
    ("deliberate_detach_blocks_strict_recovery", "b"),
    ("predicate_death_ends_receive_with_typed_loss", "b"),
    ("owner_demotion_is_typed_on_disposition", "b"),
    ("collision_evidence_names_owner_and_epoch", "b"),
    ("unknown_membership_reason_projects_without_collapse", "b"),
    ("multi_address_attach_is_atomic_or_compensable", "c"),
    (
        "compensation_distinguishes_detach_reattach_and_idempotent",
        "c",
    ),
    (
        "lifecycle_cancellation_partitions_uncertain_and_untouched",
        "c",
    ),
    ("crash_continuation_reattaches_after_daemon_restart", "c"),
    ("send_only_membership_has_no_inbound_attendance", "d"),
    ("bidirectional_receive_binds_exact_delivery_and_ack", "d"),
    ("receipt_axes_are_independent_after_durable_acceptance", "e"),
    ("ack_after_durable_ingest_survives_restart", "e"),
    ("operation_replay_is_retry_stable_and_input_sensitive", "f"),
    ("not_recorded_is_exact_and_retention_boundary_is_typed", "f"),
    ("accepted_send_indeterminate_window_exposes_recovery", "f"),
    ("post_restart_reconciliation_maps_pending_operation", "f"),
    (
        "history_filters_apply_before_bounds_and_require_attachment",
        "g",
    ),
    ("source_resolution_is_store_scoped_and_fails_closed", "g"),
    ("delta_pages_are_monotonic_and_gaps_require_resync", "h"),
    ("compound_prerequisites_order_and_fence_terminal_steps", "i"),
    ("compound_partial_outcomes_survive_restart", "i"),
    ("schema_migration_and_newer_schema_refusal", "j"),
    ("bounded_cleanup_retention_generations_and_provenance", "j"),
    ("public_evidence_redacts_paths_credentials_and_frames", "j"),
];

const FAMILIES: [&str; 10] = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];

fn assert_coverage(executed: &[&'static str]) {
    let mapped: BTreeSet<&str> = COVERAGE.iter().map(|(name, _)| *name).collect();
    let ran: BTreeSet<&str> = executed.iter().copied().collect();
    assert_eq!(
        mapped, ran,
        "coverage map and executed scenarios must match exactly"
    );
    let claimed: BTreeSet<&str> = COVERAGE
        .iter()
        .flat_map(|(_, families)| families.split(','))
        .collect();
    for family in FAMILIES {
        assert!(
            claimed.contains(family),
            "issue family {family} is not claimed by any scenario"
        );
    }
}

// ----------------------------------------------------------------------------------------
// Backend leg abstraction (isolated setup only)
// ----------------------------------------------------------------------------------------

/// One scenario's private store presentation.
#[derive(Clone, Debug)]
struct CaseStore {
    /// Named backend profile both the client and the daemon resolve.
    profile: String,
    /// SQLite file path, when this leg is SQLite.
    #[allow(dead_code)]
    sqlite_path: Option<String>,
    /// Postgres schema, when this leg is Postgres.
    #[allow(dead_code)]
    pg_schema: Option<String>,
}

/// Backend-specific isolated setup. Never an assertion surface.
#[async_trait::async_trait]
trait Leg: Send + Sync {
    fn name(&self) -> &'static str;
    /// Create a fresh, empty store for one scenario and register its profile.
    async fn prepare(&self, scenario: &str) -> CaseStore;
    /// Open a private backend handle for state/fault induction.
    async fn open_backend(&self, store: &CaseStore) -> Arc<dyn Backend>;
    /// Create an actual v2 store that omits every v3 table and column.
    async fn prepare_v2_store(&self, scenario: &str) -> CaseStore;
    /// Record a schema version newer than this build supports.
    async fn record_future_schema(&self, store: &CaseStore);
    /// Strings that must never appear in public evidence for this leg.
    fn secret_markers(&self, store: &CaseStore) -> Vec<String>;
    /// Drop the scenario store.
    async fn discard(&self, store: &CaseStore);
}

/// Shared per-leg profile registry: the config file both the in-process client
/// and the spawned daemon resolve backends from.
struct Profiles {
    config_path: std::path::PathBuf,
    entries: Mutex<BTreeMap<String, BackendProfile>>,
}

impl Profiles {
    fn new(config_path: std::path::PathBuf) -> Self {
        Self {
            config_path,
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    fn register(&self, name: &str, profile: BackendProfile) {
        let mut entries = self.entries.lock().unwrap();
        entries.insert(name.to_string(), profile);
        let config = ConfigFile {
            default: None,
            backends: entries.clone(),
        };
        std::fs::write(
            &self.config_path,
            toml::to_string_pretty(&config).expect("serialize backend profiles"),
        )
        .expect("write backend profiles");
    }

    fn get(&self, name: &str) -> BackendProfile {
        self.entries
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .expect("registered profile")
    }
}

// ----------------------------------------------------------------------------------------
// Scenario context
// ----------------------------------------------------------------------------------------

/// Everything one scenario may touch. `client*` is the public contract under
/// test; `backend`, `daemon_request`, and `restart_daemon` are isolated
/// setup/fault induction only.
struct Ctx<'a> {
    iso: &'a Isolation,
    leg: &'a dyn Leg,
    profiles: &'a Profiles,
    scenario: &'static str,
    store: CaseStore,
}

impl<'a> Ctx<'a> {
    async fn new(
        iso: &'a Isolation,
        leg: &'a dyn Leg,
        profiles: &'a Profiles,
        scenario: &'static str,
    ) -> Ctx<'a> {
        let store = leg.prepare(scenario).await;
        Ctx {
            iso,
            leg,
            profiles,
            scenario,
            store,
        }
    }

    fn bootstrap(&self) -> ApplicationDaemonBootstrap {
        ApplicationDaemonBootstrap::InstalledCurrent {
            trusted_root: self.iso.trusted_root(),
        }
    }

    /// Connect through the production `InstalledCurrent` seam with the store
    /// selected by an explicit backend-profile name.
    async fn client(&self, responsibility: &str) -> ApplicationClient {
        ApplicationClient::connect_with_daemon(
            ApplicationClientConfig {
                responsibility: ApplicationResponsibility(responsibility.to_string()),
                backend: Some(self.store.profile.clone()),
                db_override: None,
            },
            self.bootstrap(),
        )
        .await
        .unwrap_or_else(|e| panic!("{}: connect {responsibility}: {e}", self.scenario))
    }

    /// Connect to the *same* store through a different presentation: ambient
    /// `TELEX_BACKEND` selection instead of an explicit profile argument.
    async fn client_alt_presentation(&self, responsibility: &str) -> ApplicationClient {
        std::env::set_var("TELEX_BACKEND", &self.store.profile);
        let result = ApplicationClient::connect_with_daemon(
            ApplicationClientConfig {
                responsibility: ApplicationResponsibility(responsibility.to_string()),
                backend: None,
                db_override: None,
            },
            self.bootstrap(),
        )
        .await;
        std::env::remove_var("TELEX_BACKEND");
        result.unwrap_or_else(|e| panic!("{}: alt-presentation connect: {e}", self.scenario))
    }

    async fn maintenance(&self) -> ApplicationStoreMaintenance {
        ApplicationStoreMaintenance::connect(Some(&self.store.profile), None)
            .await
            .unwrap_or_else(|e| panic!("{}: maintenance connect: {e}", self.scenario))
    }

    /// Private backend handle used strictly to induce durable state or faults.
    async fn backend(&self) -> Arc<dyn Backend> {
        self.leg.open_backend(&self.store).await
    }

    fn store_key(&self) -> String {
        telex::profiles::store_key(&self.profiles.get(&self.store.profile), None)
    }

    fn address(&self, suffix: &str) -> String {
        format!("app:{}:{suffix}", self.scenario.replace('_', "-"))
    }

    fn spec(&self, suffix: &str, capability: ApplicationCapability) -> AddressSpec {
        AddressSpec {
            address: self.address(suffix),
            capability,
            description: Some(format!("{} {suffix}", self.scenario)),
            scope: None,
            tags: None,
        }
    }

    /// Privileged daemon request used only to induce a fault (session end).
    ///
    /// Runs through the installed binary rather than an in-process daemon
    /// connection: `connect_existing` authenticates the peer image against the
    /// *calling* executable, which a test binary is not.
    async fn induce_session_end(&self, runtime_id: &str) {
        let mut command = self.iso.command();
        command
            // The harness sets `TELEX_DB` for default-store isolation; the
            // scenario store is selected by profile, so the ambient override
            // must not reinterpret it.
            .env_remove("TELEX_DB")
            .args([
                "--json",
                "--backend",
                &self.store.profile,
                "daemon",
                "session-end",
                "--session",
                runtime_id,
            ]);
        let output = isolation::run_with_timeout(command, Duration::from_secs(20));
        output.assert_success("session-end fault induction");
    }

    /// Stop the daemon process and bring a fresh one up.
    ///
    /// Scenarios need to observe a *restarted* peer, not an absent one, so the
    /// replacement daemon is started through the only public spawning seam
    /// (attach) using a throwaway harness address.
    async fn restart_daemon(&self) {
        self.iso.stop_daemon();
        let starter = self.client("harness").await;
        let spec = self.spec("harness-restart", ApplicationCapability::SendOnly);
        let outcome = starter.attach(std::slice::from_ref(&spec)).await;
        assert!(
            outcome.ready,
            "the restarted daemon must accept an attach: {outcome:?}"
        );
        starter
            .detach(&spec.address)
            .await
            .expect("release the harness address");
    }

    async fn finish(self) {
        self.leg.discard(&self.store).await;
    }
}

fn send_request(operation: &str, sender: &str, to: &str, body: &str) -> SendRequest {
    SendRequest {
        operation_id: OperationId(operation.to_string()),
        sender: sender.to_string(),
        to: to.to_string(),
        cc: Vec::new(),
        kind: "note".to_string(),
        attention: "background".to_string(),
        requires_disposition: true,
        subject: Some("subject".to_string()),
        body: body.to_string(),
        metadata: Some(r#"{"probe":true}"#.to_string()),
        retry_budget: 1,
    }
}

async fn attach_one(client: &ApplicationClient, spec: &AddressSpec) {
    let outcome = client.attach(std::slice::from_ref(spec)).await;
    assert!(
        outcome.ready,
        "attach {} must succeed: {outcome:?}",
        spec.address
    );
}

// ----------------------------------------------------------------------------------------
// (a) Identity
// ----------------------------------------------------------------------------------------

async fn identity_is_fresh_stable_and_presentation_independent(ctx: &Ctx<'_>) {
    let first = ctx.client("watcher").await;
    let second = ctx.client("watcher").await;
    assert_ne!(
        first.runtime_id(),
        second.runtime_id(),
        "each connect must mint a fresh RuntimeId"
    );
    assert_eq!(
        first.responsibility(),
        second.responsibility(),
        "ApplicationResponsibility is caller-declared and stable"
    );
    assert_eq!(
        first.logical_store_id(),
        second.logical_store_id(),
        "LogicalStoreId is a property of the store, not of the runtime"
    );

    // Same store reached through a different presentation (ambient selection
    // instead of an explicit profile argument).
    let alt = ctx.client_alt_presentation("watcher").await;
    assert_eq!(alt.logical_store_id(), first.logical_store_id());
    assert_ne!(alt.runtime_id(), first.runtime_id());

    let spec = ctx.spec("identity", ApplicationCapability::Bidirectional);
    attach_one(&first, &spec).await;
    let handle = match first
        .attach(std::slice::from_ref(&spec))
        .await
        .results
        .remove(&spec.address)
    {
        Some(AddressLifecycleResult::Attached(handle)) => handle,
        other => panic!("expected an attached handle, got {other:?}"),
    };
    assert_eq!(&handle.logical_store_id, first.logical_store_id());
    assert_eq!(&handle.responsibility, first.responsibility());
    assert_eq!(&handle.runtime_id, first.runtime_id());
    assert_eq!(handle.capability, ApplicationCapability::Bidirectional);

    // Reconnect: identity survives a new client over the same store.
    drop(first);
    let reconnected = ctx.client("watcher").await;
    assert_eq!(reconnected.logical_store_id(), alt.logical_store_id());
    let LogicalStoreId(store_id) = reconnected.logical_store_id();
    assert!(
        store_id.starts_with("store-v1-"),
        "logical store identity must be an opaque store token, got {store_id}"
    );
    for marker in ctx.leg.secret_markers(&ctx.store) {
        assert!(
            !store_id.contains(&marker),
            "logical store identity must not embed connection material"
        );
    }
}

// ----------------------------------------------------------------------------------------
// (b) Typed loss, recovery, collision
// ----------------------------------------------------------------------------------------

async fn strict_recovery_refuses_repair_and_bounded_repair_reattaches(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let spec = ctx.spec("strict", ApplicationCapability::Bidirectional);
    attach_one(&client, &spec).await;

    // A healthy membership is confirmed, not repaired, under Strict.
    let confirmed = client
        .reconcile(&spec, RecoveryPolicy::Strict)
        .await
        .expect("strict reconcile confirms a live membership");
    assert_eq!(confirmed.address, spec.address);

    // Fault induction: the daemon restarts, so the runtime's membership is gone
    // while the durable lease still records this client as the owner.
    ctx.restart_daemon().await;

    match client.reconcile(&spec, RecoveryPolicy::Strict).await {
        Err(ApplicationClientError::MembershipLost {
            address,
            reason: MembershipLossReason::NeedsAttach,
            ..
        }) => assert_eq!(address, spec.address),
        other => panic!("strict recovery must refuse to repair, got {other:?}"),
    }

    let repaired = client
        .reconcile(&spec, RecoveryPolicy::BoundedRepair { retries: 3 })
        .await
        .expect("bounded repair re-attaches within its budget");
    assert_eq!(repaired.address, spec.address);
    assert_eq!(&repaired.runtime_id, client.runtime_id());

    // Strict recovery over a durably re-owned address is refused with typed
    // collision evidence rather than a silent takeover.
    ctx.restart_daemon().await;
    let backend = ctx.backend().await;
    backend
        .reset_epoch_lease(&spec.address)
        .await
        .expect("reset epoch lease");
    backend
        .claim_epoch_lease(&spec.address, "foreign-owner", 60)
        .await
        .expect("foreign epoch claim");
    match client.reconcile(&spec, RecoveryPolicy::Strict).await {
        Err(ApplicationClientError::Collision(evidence)) => {
            assert_eq!(evidence.address, spec.address);
            assert_eq!(evidence.owner_instance_id.as_deref(), Some("foreign-owner"));
            assert!(evidence.lease_epoch.is_some());
        }
        other => panic!("strict recovery must refuse a re-owned address, got {other:?}"),
    }
}

async fn restart_loses_membership_and_reports_typed_loss(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let spec = ctx.spec("restart", ApplicationCapability::Bidirectional);
    attach_one(&client, &spec).await;

    ctx.restart_daemon().await;

    let health = client.health().await.expect("health after restart");
    let record = health
        .iter()
        .find(|record| record.address == spec.address)
        .expect("health record for the attached address");
    assert!(
        !record.registered && record.stopped_or_unattended,
        "a restarted daemon must not report the old membership as registered: {record:?}"
    );

    match client.receive(&spec.address, Some(300)).await {
        Err(ApplicationClientError::MembershipLost { reason, .. }) => assert!(
            matches!(
                reason,
                MembershipLossReason::DaemonRestart | MembershipLossReason::NeedsAttach
            ),
            "restart loss must be typed, got {reason:?}"
        ),
        other => panic!("receive after restart must report typed loss, got {other:?}"),
    }

    let outcome = client
        .reconcile_many(
            std::slice::from_ref(&spec),
            RecoveryPolicy::BoundedRepair { retries: 3 },
        )
        .await;
    assert!(outcome.ready, "bounded repair must recover: {outcome:?}");
}

async fn deliberate_detach_blocks_strict_recovery(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let spec = ctx.spec("detach", ApplicationCapability::Bidirectional);
    attach_one(&client, &spec).await;

    client
        .detach(&spec.address)
        .await
        .expect("deliberate detach");

    match client.reconcile(&spec, RecoveryPolicy::Strict).await {
        Err(ApplicationClientError::MembershipLost {
            reason: MembershipLossReason::NeedsAttach,
            ..
        }) => {}
        other => panic!("detached address is no longer a local membership: {other:?}"),
    }

    // A second client observing the same store sees the durable intent, and
    // strict recovery refuses to repair across it.
    let observer = ctx.client("station").await;
    attach_one(&observer, &spec).await;
    observer
        .detach(&spec.address)
        .await
        .expect("second deliberate detach");
    let health = observer.health().await.expect("health after detach");
    let detached = health
        .iter()
        .find(|record| record.address == spec.address)
        .expect("detached address still reports health");
    assert!(!detached.registered && detached.stopped_or_unattended);
    assert!(
        detached.lifecycle.iter().any(|evidence| matches!(
            evidence,
            telex::application_client::ApplicationLifecycleEvidence::DeliberateDetach { .. }
        )),
        "deliberate detach must be durable public evidence: {detached:?}"
    );
}

async fn predicate_death_ends_receive_with_typed_loss(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let spec = ctx.spec("predicate", ApplicationCapability::Bidirectional);
    attach_one(&client, &spec).await;

    // Fault induction: the daemon ends this runtime's presence.
    ctx.induce_session_end(&client.runtime_id().0).await;

    match client.receive(&spec.address, Some(500)).await {
        Err(ApplicationClientError::MembershipLost {
            address,
            reason: MembershipLossReason::PredicateDeath,
            ..
        }) => assert_eq!(address, spec.address),
        other => panic!("ended presence must project PredicateDeath, got {other:?}"),
    }
}

async fn owner_demotion_is_typed_on_disposition(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let sender = ctx.spec("demote-sender", ApplicationCapability::Bidirectional);
    let station = ctx.spec("demote-station", ApplicationCapability::Bidirectional);
    attach_one(&client, &sender).await;
    attach_one(&client, &station).await;

    let request = send_request(
        "demote-op",
        &sender.address,
        &station.address,
        "demotion probe",
    );
    client.send(request).await.expect("send");
    let delivery = client
        .receive(&station.address, Some(2000))
        .await
        .expect("receive")
        .expect("a delivery is available");

    // State induction: a foreign occupant takes the durable epoch lease.
    let backend = ctx.backend().await;
    backend
        .reset_epoch_lease(&station.address)
        .await
        .expect("reset epoch lease");
    backend
        .claim_epoch_lease(&station.address, "foreign-owner", 60)
        .await
        .expect("foreign epoch claim");

    match client
        .disposition(&sender.address, &delivery.delivery, "handled", None)
        .await
    {
        Err(ApplicationClientError::MembershipLost {
            address,
            reason: MembershipLossReason::OwnerDemoted,
            ..
        }) => assert_eq!(address, station.address),
        other => panic!("stale owner must project OwnerDemoted, got {other:?}"),
    }
}

async fn collision_evidence_names_owner_and_epoch(ctx: &Ctx<'_>) {
    let owner = ctx.client("station").await;
    let rival = ctx.client("station").await;
    let spec = ctx.spec("collision", ApplicationCapability::Bidirectional);
    attach_one(&owner, &spec).await;

    let outcome = rival.attach(std::slice::from_ref(&spec)).await;
    assert!(!outcome.ready, "a colliding attach must not be ready");
    match outcome.results.get(&spec.address) {
        Some(AddressLifecycleResult::Failed(ApplicationClientError::Collision(evidence))) => {
            assert_eq!(evidence.address, spec.address);
            assert!(
                evidence.owner_instance_id.is_some(),
                "collision evidence must name the durable owner: {evidence:?}"
            );
            assert!(
                evidence.lease_epoch.is_some(),
                "collision evidence must carry the lease epoch: {evidence:?}"
            );
            assert!(!evidence.guidance.is_empty());
        }
        other => panic!("expected typed collision evidence, got {other:?}"),
    }
}

async fn unknown_membership_reason_projects_without_collapse(ctx: &Ctx<'_>) {
    // A future wire reason cannot be induced through a production daemon
    // without a fault-injection seam, and none is added here. The wire->typed
    // mapping is covered deterministically by the crate unit tests
    // (`unknown_wire_membership_reason_preserves_exact_token`); this scenario
    // proves the *public projection* is lossless and does not collapse known
    // reasons, on both backend legs.
    let unknown = MembershipLossReason::Unknown {
        raw_reason: Some("future-loss-reason".to_string()),
    };
    let encoded = serde_json::to_string(&unknown).expect("public reason serializes");
    let decoded: MembershipLossReason =
        serde_json::from_str(&encoded).expect("public reason round-trips");
    assert_eq!(decoded, unknown, "raw future evidence must survive intact");

    let known = [
        MembershipLossReason::DaemonRestart,
        MembershipLossReason::PredicateDeath,
        MembershipLossReason::Collision,
        MembershipLossReason::DeliberateDetach,
        MembershipLossReason::NeedsAttach,
        MembershipLossReason::OwnerDemoted,
    ];
    let projected: BTreeSet<String> = known
        .iter()
        .map(|reason| serde_json::to_string(reason).expect("serialize"))
        .collect();
    assert_eq!(
        projected.len(),
        known.len(),
        "known loss reasons must not collapse onto each other"
    );
    assert!(
        !projected.contains(&encoded),
        "an unknown reason must not be projected as a known one"
    );

    // Ambiguity is a declared, fail-closed public outcome; assert its public
    // projection is available to consumers on both legs.
    let ambiguous =
        ApplicationClientError::AmbiguousSender(vec![ctx.address("amb-a"), ctx.address("amb-b")]);
    let text = ambiguous.to_string();
    assert!(text.contains("ambiguous sender"), "{text}");
}

// ----------------------------------------------------------------------------------------
// (c) Multi-address lifecycle: atomic-or-compensable, cancellation, continuation
// ----------------------------------------------------------------------------------------

async fn multi_address_attach_is_atomic_or_compensable(ctx: &Ctx<'_>) {
    let rival = ctx.client("station").await;
    let contested = ctx.spec("multi-contested", ApplicationCapability::Bidirectional);
    attach_one(&rival, &contested).await;

    let client = ctx.client("station").await;
    let first = ctx.spec("multi-a", ApplicationCapability::Bidirectional);
    let last = ctx.spec("multi-c", ApplicationCapability::SendOnly);
    let specs = vec![first.clone(), contested.clone(), last.clone()];

    let outcome = client.attach(&specs).await;
    assert!(
        !outcome.ready,
        "a partially blocked attach is not ready: {outcome:?}"
    );
    assert!(matches!(
        outcome.results.get(&first.address),
        Some(AddressLifecycleResult::Attached(_))
    ));
    assert!(matches!(
        outcome.results.get(&contested.address),
        Some(AddressLifecycleResult::Failed(
            ApplicationClientError::Collision(_)
        ))
    ));
    assert!(matches!(
        outcome.results.get(&last.address),
        Some(AddressLifecycleResult::Attached(_))
    ));

    // Not atomic on the wire, so it must be exactly compensable: every address
    // this operation newly attached carries a Detach compensation handle.
    let compensable: BTreeSet<&str> = outcome
        .compensation
        .iter()
        .filter(|handle| matches!(handle.action, CompensationAction::Detach))
        .map(|handle| handle.address.as_str())
        .collect();
    assert_eq!(
        compensable,
        BTreeSet::from([first.address.as_str(), last.address.as_str()]),
        "compensation must cover exactly the newly attached addresses: {:?}",
        outcome.compensation
    );
    for handle in &outcome.compensation {
        assert_eq!(&handle.runtime_id, client.runtime_id());
    }

    // Duplicate addresses fail validation before any work is attempted.
    let duplicates = vec![first.clone(), first.clone()];
    let rejected = client.attach(&duplicates).await;
    assert!(rejected.results.is_empty() && rejected.compensation.is_empty());
    assert!(matches!(
        rejected.validation_error,
        Some(ApplicationClientError::InvalidRequest(_))
    ));

    // An empty address set is a no-op, not an error.
    let empty = client.attach(&[]).await;
    assert!(empty.ready && empty.results.is_empty() && empty.validation_error.is_none());
}

async fn compensation_distinguishes_detach_reattach_and_idempotent(ctx: &Ctx<'_>) {
    let rival = ctx.client("station").await;
    let blocker = ctx.spec("comp-blocked", ApplicationCapability::Bidirectional);
    attach_one(&rival, &blocker).await;

    let client = ctx.client("station").await;
    let mut spec = ctx.spec("comp-target", ApplicationCapability::Bidirectional);

    // New attachment -> Detach compensates it.
    let outcome = client.attach(&[spec.clone(), blocker.clone()]).await;
    assert!(!outcome.ready);
    let handle = outcome
        .compensation
        .iter()
        .find(|handle| handle.address == spec.address)
        .expect("compensation for the newly attached address");
    assert!(matches!(handle.action, CompensationAction::Detach));

    // Changed spec over an existing membership -> Reattach(previous spec).
    let previous = spec.clone();
    spec.description = Some("changed description".to_string());
    let outcome = client.attach(&[spec.clone(), blocker.clone()]).await;
    assert!(!outcome.ready);
    let handle = outcome
        .compensation
        .iter()
        .find(|handle| handle.address == spec.address)
        .expect("compensation for the changed attachment");
    match &handle.action {
        CompensationAction::Reattach(restored) => assert_eq!(
            restored, &previous,
            "compensation must restore the exact previous spec"
        ),
        other => panic!("expected Reattach(previous_spec), got {other:?}"),
    }

    // Identical re-attach -> idempotent, no compensation is owed.
    let outcome = client.attach(&[spec.clone(), blocker.clone()]).await;
    assert!(!outcome.ready);
    assert!(
        !outcome
            .compensation
            .iter()
            .any(|handle| handle.address == spec.address),
        "an idempotent re-attach owes no compensation: {:?}",
        outcome.compensation
    );

    // Detach compensates by reattaching the previous spec.
    let outcome = client
        .detach_many(&[spec.address.clone(), blocker.address.clone()])
        .await;
    assert!(!outcome.ready, "detaching a foreign address must fail");
    let handle = outcome
        .compensation
        .iter()
        .find(|handle| handle.address == spec.address)
        .expect("compensation for the detached address");
    match &handle.action {
        CompensationAction::Reattach(restored) => assert_eq!(restored, &spec),
        other => panic!("expected Reattach(previous_spec) after detach, got {other:?}"),
    }
    assert!(matches!(
        outcome.results.get(&spec.address),
        Some(AddressLifecycleResult::Detached(_))
    ));

    // A fully successful operation owes nothing.
    let ok = client.attach(std::slice::from_ref(&spec)).await;
    assert!(ok.ready && ok.compensation.is_empty());
}

async fn lifecycle_cancellation_partitions_uncertain_and_untouched(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let a = ctx.spec("cancel-a", ApplicationCapability::Bidirectional);
    let b = ctx.spec("cancel-b", ApplicationCapability::Bidirectional);
    let c = ctx.spec("cancel-c", ApplicationCapability::Bidirectional);
    let specs = vec![a.clone(), b.clone(), c.clone()];

    // Cancelled before any work: nothing attempted, nothing uncertain.
    let untouched = client.begin_attach(&specs).cancelled_outcome();
    let evidence = untouched.cancellation.expect("cancellation evidence");
    assert_eq!(evidence.operation, LifecycleOperationKind::Attach);
    assert_eq!(evidence.may_have_committed, None);
    assert_eq!(
        evidence.not_attempted,
        vec![a.address.clone(), b.address.clone(), c.address.clone()]
    );

    // Cancelled with an in-flight request: the in-flight address may have
    // committed and must be reconciled, the rest were never attempted.
    let mut operation = client.begin_attach(&specs);
    let waker = Waker::noop();
    let mut task_context = Context::from_waker(waker);
    let mut advance = Box::pin(operation.advance());
    assert!(
        matches!(advance.as_mut().poll(&mut task_context), Poll::Pending),
        "the first poll must reach the asynchronous request boundary"
    );
    drop(advance);
    let outcome = operation.cancelled_outcome();
    let evidence = outcome.cancellation.expect("cancellation evidence");
    assert_eq!(
        evidence.may_have_committed.as_deref(),
        Some(a.address.as_str())
    );
    assert_eq!(
        evidence.not_attempted,
        vec![b.address.clone(), c.address.clone()]
    );
    assert!(
        !outcome.results.contains_key(&a.address),
        "an uncertain address has no terminal result"
    );
    assert!(!outcome.ready);

    // The uncertain address is reconcilable rather than blindly retryable.
    let repaired = client
        .reconcile(&a, RecoveryPolicy::BoundedRepair { retries: 3 })
        .await
        .expect("reconcile the uncertain address");
    assert_eq!(repaired.address, a.address);

    // Every operation kind partitions the same way.
    attach_one(&client, &b).await;
    let detach = client
        .begin_detach_many(&[b.address.clone(), c.address.clone()])
        .cancelled_outcome();
    let evidence = detach.cancellation.expect("detach cancellation evidence");
    assert_eq!(evidence.operation, LifecycleOperationKind::Detach);
    assert_eq!(
        evidence.not_attempted,
        vec![b.address.clone(), c.address.clone()]
    );
    let reconcile = client
        .begin_reconcile_many(&specs, RecoveryPolicy::Strict)
        .cancelled_outcome();
    let evidence = reconcile.cancellation.expect("reconcile cancellation");
    assert_eq!(evidence.operation, LifecycleOperationKind::Reconcile);
}

async fn crash_continuation_reattaches_after_daemon_restart(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let a = ctx.spec("continue-a", ApplicationCapability::Bidirectional);
    let b = ctx.spec("continue-b", ApplicationCapability::SendOnly);
    let outcome = client.attach(&[a.clone(), b.clone()]).await;
    assert!(outcome.ready, "initial attach: {outcome:?}");

    ctx.restart_daemon().await;

    // Continuation after a daemon crash/restart: the same declarative set is
    // re-applied and every address is reconciled, with typed results.
    let outcome = client
        .reconcile_many(
            &[a.clone(), b.clone()],
            RecoveryPolicy::BoundedRepair { retries: 3 },
        )
        .await;
    assert!(
        outcome.ready,
        "crash continuation must reattach the whole set: {outcome:?}"
    );
    for spec in [&a, &b] {
        match outcome.results.get(&spec.address) {
            Some(AddressLifecycleResult::Reconciled(handle)) => {
                assert_eq!(handle.capability, spec.capability);
                assert_eq!(&handle.runtime_id, client.runtime_id());
            }
            other => panic!(
                "expected a reconciled handle for {}, got {other:?}",
                spec.address
            ),
        }
    }
    assert!(
        outcome.compensation.is_empty(),
        "a ready continuation owes no compensation"
    );
}

// ----------------------------------------------------------------------------------------
// (d) Send-only vs bidirectional
// ----------------------------------------------------------------------------------------

async fn send_only_membership_has_no_inbound_attendance(ctx: &Ctx<'_>) {
    let client = ctx.client("watcher").await;
    let watcher = ctx.spec("sendonly", ApplicationCapability::SendOnly);
    attach_one(&client, &watcher).await;

    let station = ctx.spec("sendonly-peer", ApplicationCapability::Bidirectional);
    let peer = ctx.client("station").await;
    attach_one(&peer, &station).await;

    // Send-only membership sends.
    let result = client
        .send(send_request(
            "sendonly-op",
            &watcher.address,
            &station.address,
            "watcher output",
        ))
        .await
        .expect("send-only membership can send");
    assert_eq!(result.recipient, station.address);

    // ... and refuses every inbound seam, so it can never create false
    // attendance for its own address.
    assert!(matches!(
        client.receive(&watcher.address, Some(100)).await,
        Err(ApplicationClientError::UnsupportedCapability(_))
    ));
    assert!(matches!(
        client
            .history(Some(watcher.address.clone()), false, None, None, None, 10)
            .await,
        Err(ApplicationClientError::UnsupportedCapability(_))
    ));

    let health = client.health().await.expect("health");
    let record = health
        .iter()
        .find(|record| record.address == watcher.address)
        .expect("send-only health record");
    assert_eq!(record.capability, ApplicationCapability::SendOnly);
    assert!(record.registered && record.sender_ready);
    assert!(
        !record.receive_ready && !record.attended_but_deaf,
        "send-only membership must never look inbound-attended: {record:?}"
    );
    assert_eq!(record.pending_unconsumed, 0);
    assert_eq!(record.inbound_actionable, 0);
    assert!(!record.acknowledgment_pending && record.outstanding_ack_count == 0);

    // The recipient's own inbound attendance is unaffected.
    let peer_health = peer.health().await.expect("peer health");
    let peer_record = peer_health
        .iter()
        .find(|record| record.address == station.address)
        .expect("bidirectional health record");
    assert!(peer_record.registered);
    assert_eq!(peer_record.capability, ApplicationCapability::Bidirectional);
}

async fn bidirectional_receive_binds_exact_delivery_and_ack(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let sender = ctx.spec("exact-sender", ApplicationCapability::SendOnly);
    let station = ctx.spec("exact-station", ApplicationCapability::Bidirectional);
    attach_one(&client, &sender).await;
    attach_one(&client, &station).await;

    let sent = client
        .send(send_request(
            "exact-op",
            &sender.address,
            &station.address,
            "exact delivery",
        ))
        .await
        .expect("send");

    let delivery = client
        .receive(&station.address, Some(2000))
        .await
        .expect("receive")
        .expect("a delivery is available");
    assert_eq!(delivery.delivery.message_id, sent.message_id);
    assert_eq!(delivery.delivery.recipient, station.address);
    assert_eq!(
        &delivery.delivery.logical_store_id,
        client.logical_store_id()
    );
    assert_eq!(delivery.from.as_deref(), Some(sender.address.as_str()));
    assert_eq!(delivery.primary_to, station.address);
    assert!(delivery.requires_disposition);

    // The exact durable delivery row is the identity the ack is bound to.
    let backend = ctx.backend().await;
    let row = backend
        .delivery_for_recipient(sent.message_id, &station.address)
        .await
        .expect("delivery lookup")
        .expect("durable delivery row");
    assert_eq!(delivery.delivery.delivery_id, row.id);

    // A different delivery row identity is refused, not silently applied.
    let mut wrong = delivery.delivery.clone();
    wrong.delivery_id = row.id + 1;
    assert!(matches!(
        client
            .disposition(&sender.address, &wrong, "acknowledged", None)
            .await,
        Err(ApplicationClientError::DeliveryMismatch { .. })
    ));
    let mut foreign_store = delivery.delivery.clone();
    foreign_store.logical_store_id = LogicalStoreId("store-v1-foreign".to_string());
    assert!(matches!(
        client
            .disposition(&sender.address, &foreign_store, "acknowledged", None)
            .await,
        Err(ApplicationClientError::DeliveryMismatch { .. })
    ));

    assert_eq!(
        client.acknowledge(&delivery.ack).await.expect("ack"),
        AckResult::Marked
    );
    assert!(matches!(
        client.acknowledge(&delivery.ack).await.expect("re-ack"),
        AckResult::AlreadyConsumed | AckResult::NoDelivery
    ));

    // Receive is not available on a send-only address even after a real
    // bidirectional receive on the same client.
    assert!(matches!(
        client.receive(&sender.address, Some(100)).await,
        Err(ApplicationClientError::UnsupportedCapability(_))
    ));
}

// ----------------------------------------------------------------------------------------
// (e) Independent receipt axes and ack-after-ingest
// ----------------------------------------------------------------------------------------

async fn receipt_axes_are_independent_after_durable_acceptance(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let sender = ctx.spec("axes-sender", ApplicationCapability::SendOnly);
    let station = ctx.spec("axes-station", ApplicationCapability::Bidirectional);
    attach_one(&client, &sender).await;
    attach_one(&client, &station).await;

    let request = send_request("axes-op", &sender.address, &station.address, "axes probe");
    let recovery = client.prepare_send(&request).await.expect("prepare send");
    let sent = client.send(request).await.expect("send");

    // Durable acceptance is decided independently of occupancy, push,
    // consumption and workflow disposition.
    assert_eq!(sent.axes.durable_acceptance, EvidenceState::Accepted);
    assert!(
        sent.axes.occupied_at_acceptance.is_some(),
        "occupancy at acceptance is reported as its own axis"
    );
    assert_eq!(sent.axes.push_acceptance, EvidenceState::Unknown);
    assert_eq!(sent.axes.recipient_consumption, EvidenceState::Unknown);
    assert_eq!(sent.axes.workflow_disposition, EvidenceState::Unknown);
    assert!(!sent.replayed);

    let axes = client
        .refresh_receipt_axes(&recovery)
        .await
        .expect("refresh axes before consumption");
    assert_eq!(axes.durable_acceptance, EvidenceState::Accepted);
    assert_eq!(
        axes.push_acceptance,
        EvidenceState::Unavailable,
        "push acceptance is not durable evidence"
    );
    assert_eq!(axes.recipient_consumption, EvidenceState::Pending);
    assert_eq!(axes.workflow_disposition, EvidenceState::NotAttempted);

    let delivery = client
        .receive(&station.address, Some(2000))
        .await
        .expect("receive")
        .expect("delivery");
    let axes = client
        .refresh_receipt_axes(&recovery)
        .await
        .expect("refresh axes after receive");
    assert_eq!(
        axes.recipient_consumption,
        EvidenceState::Pending,
        "receiving is not consumption; only a durable ack is"
    );

    assert_eq!(
        client.acknowledge(&delivery.ack).await.expect("ack"),
        AckResult::Marked
    );
    let axes = client
        .refresh_receipt_axes(&recovery)
        .await
        .expect("refresh axes after ack");
    assert_eq!(axes.recipient_consumption, EvidenceState::Accepted);
    assert_eq!(
        axes.workflow_disposition,
        EvidenceState::NotAttempted,
        "consumption does not imply a workflow disposition"
    );

    client
        .disposition(&sender.address, &delivery.delivery, "handled", Some("done"))
        .await
        .expect("terminal disposition");
    let axes = client
        .refresh_receipt_axes(&recovery)
        .await
        .expect("refresh axes after disposition");
    assert_eq!(
        axes.workflow_disposition,
        EvidenceState::Disposition("handled".to_string())
    );
    assert_eq!(axes.durable_acceptance, EvidenceState::Accepted);
}

async fn ack_after_durable_ingest_survives_restart(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let sender = ctx.spec("ingest-sender", ApplicationCapability::SendOnly);
    let station = ctx.spec("ingest-station", ApplicationCapability::Bidirectional);
    attach_one(&client, &sender).await;
    attach_one(&client, &station).await;

    let request = send_request(
        "ingest-op",
        &sender.address,
        &station.address,
        "durable ingest",
    );
    let recovery = client.prepare_send(&request).await.expect("prepare");
    let sent = client.send(request).await.expect("send");
    let delivery = client
        .receive(&station.address, Some(2000))
        .await
        .expect("receive")
        .expect("delivery");

    // The caller has the delivery but has not durably ingested it yet, so the
    // consumption axis must not have advanced.
    let axes = client
        .refresh_receipt_axes(&recovery)
        .await
        .expect("axes before ack");
    assert_eq!(axes.recipient_consumption, EvidenceState::Pending);

    ctx.restart_daemon().await;

    // Recover membership, then acknowledge only after the caller's own durable
    // ingest boundary. The message is neither lost nor duplicated.
    let outcome = client
        .reconcile_many(
            std::slice::from_ref(&station),
            RecoveryPolicy::BoundedRepair { retries: 3 },
        )
        .await;
    assert!(outcome.ready, "reattach after restart: {outcome:?}");
    assert_eq!(
        client.acknowledge(&delivery.ack).await.expect("ack"),
        AckResult::Marked
    );
    let axes = client
        .refresh_receipt_axes(&recovery)
        .await
        .expect("axes after ack");
    assert_eq!(axes.recipient_consumption, EvidenceState::Accepted);

    let history = client
        .history(Some(station.address.clone()), false, None, None, None, 50)
        .await
        .expect("history");
    let matching: Vec<_> = history
        .iter()
        .filter(|item| item.message.id == sent.message_id)
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "no loss and no duplication across restart"
    );
    assert!(matching[0]
        .delivery
        .as_ref()
        .and_then(|row| row.consumed_at_ms)
        .is_some());
}

// ----------------------------------------------------------------------------------------
// (f) Retry-stable operations, evidence, retention, restart reconciliation
// ----------------------------------------------------------------------------------------

async fn operation_replay_is_retry_stable_and_input_sensitive(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let sender = ctx.spec("replay-sender", ApplicationCapability::SendOnly);
    let station = ctx.spec("replay-station", ApplicationCapability::Bidirectional);
    attach_one(&client, &sender).await;
    attach_one(&client, &station).await;

    let request = send_request("replay-op", &sender.address, &station.address, "original");
    let recovery = client.prepare_send(&request).await.expect("prepare");
    assert_eq!(recovery.operation_id, request.operation_id);
    assert_eq!(&recovery.logical_store_id, client.logical_store_id());
    assert_eq!(&recovery.responsibility, client.responsibility());
    assert!(recovery.payload_identity.comparable);
    assert_eq!(recovery.payload_identity.algorithm, "sha256");

    let first = client.send(request.clone()).await.expect("first send");
    assert!(!first.replayed);
    let replayed = client.send(request.clone()).await.expect("retry");
    assert!(replayed.replayed, "a retried operation replays its result");
    assert_eq!(replayed.message_id, first.message_id);
    assert_eq!(replayed.payload_identity, first.payload_identity);

    // Same operation id, different payload: refused with comparable evidence.
    let mut mutated = request.clone();
    mutated.body = "mutated".to_string();
    match client.send(mutated.clone()).await {
        Err(ApplicationClientError::OperationMismatch {
            operation_id,
            evidence,
        }) => {
            assert_eq!(operation_id, request.operation_id);
            assert_ne!(evidence.attempted.digest, evidence.existing.digest);
            assert!(evidence.attempted.comparable && evidence.existing.comparable);
            assert_eq!(evidence.existing.digest, first.payload_identity.digest);
        }
        other => panic!("payload change must be refused, got {other:?}"),
    }

    // Cross-store recovery handles are refused with a typed binding mismatch.
    let foreign = RecoveryHandle {
        logical_store_id: LogicalStoreId("store-v1-foreign".to_string()),
        responsibility: client.responsibility().clone(),
        operation_id: request.operation_id.clone(),
        payload_identity: recovery.payload_identity.clone(),
        retention_generation: recovery.retention_generation,
    };
    match client.reconcile_operation(&foreign).await {
        Err(ApplicationClientError::StoreBindingMismatch { staged, current }) => {
            assert_eq!(staged, foreign.logical_store_id);
            assert_eq!(&current, client.logical_store_id());
        }
        other => panic!("foreign store binding must be refused, got {other:?}"),
    }

    // The recorded operation projects as a typed public record.
    match client
        .reconcile_operation(&recovery)
        .await
        .expect("reconcile recorded operation")
    {
        OperationReconciliation::Recorded(record) => {
            assert_eq!(record.operation_id, request.operation_id);
            assert_eq!(record.sender, sender.address);
            assert_eq!(record.retry_budget, request.retry_budget);
            match record.outcome {
                RecordedOperationOutcome::Accepted(result)
                | RecordedOperationOutcome::Duplicate(result) => {
                    assert_eq!(result.message_id, first.message_id);
                }
                other => panic!("expected an accepted outcome, got {other:?}"),
            }
        }
        other => panic!("expected a recorded operation, got {other:?}"),
    }
}

async fn not_recorded_is_exact_and_retention_boundary_is_typed(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let sender = ctx.spec("retain-sender", ApplicationCapability::SendOnly);
    let station = ctx.spec("retain-station", ApplicationCapability::Bidirectional);
    attach_one(&client, &sender).await;
    attach_one(&client, &station).await;

    // A prepared-but-never-attempted operation is authoritatively NotRecorded
    // inside its own retention generation.
    let never = send_request("never-op", &sender.address, &station.address, "never sent");
    let staged = client.prepare_send(&never).await.expect("prepare");
    let staged_generation = staged
        .retention_generation
        .expect("a prepared handle carries its retention generation");
    match client
        .reconcile_operation(&staged)
        .await
        .expect("reconcile unrecorded operation")
    {
        OperationReconciliation::NotRecorded(evidence) => {
            assert_eq!(evidence.operation_id, never.operation_id);
            assert_eq!(evidence.retention_generation, staged_generation);
            assert_eq!(&evidence.logical_store_id, client.logical_store_id());
            assert_eq!(&evidence.responsibility, client.responsibility());
            assert_eq!(evidence.payload_identity, staged.payload_identity);
        }
        other => panic!("expected exact NotRecorded evidence, got {other:?}"),
    }

    // Complete an operation, then prune it so the retention generation moves.
    let recorded = send_request("retained-op", &sender.address, &station.address, "kept");
    client.send(recorded).await.expect("send");
    let report = client
        .cleanup(RetentionPolicy {
            completed_before_ms: i64::MAX,
            max_delete: 100,
        })
        .await
        .expect("cleanup");
    assert!(
        report.operations_deleted >= 1,
        "cleanup must prune the completed operation: {report:?}"
    );

    // The stale handle can no longer make an authoritative absence claim.
    match client
        .reconcile_operation(&staged)
        .await
        .expect("reconcile across the retention boundary")
    {
        OperationReconciliation::RetentionBoundaryCrossed {
            staged_generation: staged_gen,
            current_generation,
        } => {
            assert_eq!(staged_gen, Some(staged_generation));
            assert!(
                current_generation > staged_generation,
                "the retention generation must advance: {staged_generation} -> {current_generation}"
            );
        }
        other => panic!("expected RetentionBoundaryCrossed, got {other:?}"),
    }

    // A freshly prepared handle is authoritative again in the new generation.
    let fresh = client.prepare_send(&never).await.expect("re-prepare");
    assert!(fresh.retention_generation > Some(staged_generation));
    match client
        .reconcile_operation(&fresh)
        .await
        .expect("reconcile in the new generation")
    {
        OperationReconciliation::NotRecorded(evidence) => {
            assert_eq!(
                Some(evidence.retention_generation),
                fresh.retention_generation
            );
        }
        other => panic!("expected NotRecorded in the new generation, got {other:?}"),
    }
}

async fn accepted_send_indeterminate_window_exposes_recovery(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let sender = ctx.spec("indet-sender", ApplicationCapability::SendOnly);
    let station = ctx.spec("indet-station", ApplicationCapability::Bidirectional);
    attach_one(&client, &sender).await;
    attach_one(&client, &station).await;

    let request = send_request(
        "indeterminate-op",
        &sender.address,
        &station.address,
        "uncertain window",
    );
    // The prepared handle is the only public source of the payload identity, so
    // the induced durable record binds the exact same fingerprint the client
    // would compute.
    let recovery = client.prepare_send(&request).await.expect("prepare");

    // State induction: leave the operation in the durable indeterminate window
    // that an accepted-but-unconfirmed send produces.
    let backend = ctx.backend().await;
    backend
        .begin_application_operation(&NewApplicationOperation {
            logical_store_id: client.logical_store_id().0.clone(),
            application_responsibility: client.responsibility().0.clone(),
            operation_id: request.operation_id.0.clone(),
            operation_kind: "send".to_string(),
            sender: sender.address.clone(),
            recipients_json: serde_json::to_string(&(&station.address, Vec::<String>::new()))
                .expect("recipients"),
            payload_fingerprint: recovery.payload_identity.digest.clone(),
            retry_budget: request.retry_budget as i64,
            created_at_ms: now_ms(),
        })
        .await
        .expect("begin induced operation");
    backend
        .complete_application_operation(
            &client.logical_store_id().0,
            &client.responsibility().0,
            &request.operation_id.0,
            "indeterminate",
            None,
            Some(&serde_json::to_string(&recovery).expect("recovery json")),
        )
        .await
        .expect("record the indeterminate window");

    // A retry inside the window is neither silently duplicated nor reported as
    // absent: it is typed indeterminate with a usable recovery handle.
    match client.send(request.clone()).await {
        Err(ApplicationClientError::Indeterminate {
            recovery: handle, ..
        }) => {
            assert_eq!(handle.operation_id, request.operation_id);
            assert_eq!(handle.payload_identity, recovery.payload_identity);
            assert_eq!(&handle.logical_store_id, client.logical_store_id());
        }
        other => panic!("an indeterminate window must stay indeterminate, got {other:?}"),
    }

    match client
        .reconcile_operation(&recovery)
        .await
        .expect("reconcile the indeterminate operation")
    {
        OperationReconciliation::Recorded(record) => match record.outcome {
            RecordedOperationOutcome::Indeterminate {
                recovery: handle, ..
            } => {
                assert_eq!(handle.operation_id, request.operation_id);
            }
            other => panic!("expected an indeterminate outcome, got {other:?}"),
        },
        other => panic!("expected a recorded operation, got {other:?}"),
    }

    // Refreshing receipt axes on an indeterminate operation preserves the
    // recovery evidence instead of inventing an acceptance.
    match client.refresh_receipt_axes(&recovery).await {
        Err(ApplicationClientError::Indeterminate {
            recovery: handle, ..
        }) => assert_eq!(handle.operation_id, request.operation_id),
        other => panic!("axes refresh must preserve uncertainty, got {other:?}"),
    }

    // An operation with no acceptance evidence can be explicitly abandoned.
    let orphan = send_request(
        "abandon-op",
        &sender.address,
        &station.address,
        "abandon me",
    );
    let orphan_recovery = client.prepare_send(&orphan).await.expect("prepare orphan");
    backend
        .begin_application_operation(&NewApplicationOperation {
            logical_store_id: client.logical_store_id().0.clone(),
            application_responsibility: client.responsibility().0.clone(),
            operation_id: orphan.operation_id.0.clone(),
            operation_kind: "send".to_string(),
            sender: sender.address.clone(),
            recipients_json: serde_json::to_string(&(&station.address, Vec::<String>::new()))
                .expect("recipients"),
            payload_fingerprint: orphan_recovery.payload_identity.digest.clone(),
            retry_budget: 0,
            created_at_ms: now_ms(),
        })
        .await
        .expect("begin orphan operation");
    let abandoned = client
        .abandon_unmapped_operation(&orphan_recovery, "no acceptance evidence")
        .await
        .expect("abandon the unmapped operation");
    assert_eq!(abandoned.operation_id, orphan.operation_id.0);
}

async fn post_restart_reconciliation_maps_pending_operation(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let sender = ctx.spec("pending-sender", ApplicationCapability::SendOnly);
    let station = ctx.spec("pending-station", ApplicationCapability::Bidirectional);
    attach_one(&client, &sender).await;
    attach_one(&client, &station).await;

    let request = send_request(
        "pending-op",
        &sender.address,
        &station.address,
        "pending mapping",
    );
    let recovery = client.prepare_send(&request).await.expect("prepare");
    let sent = client.send(request).await.expect("send");

    // State induction: rewind the durable record to the pending state a crash
    // between daemon acceptance and local completion would leave behind.
    let backend = ctx.backend().await;
    backend
        .complete_application_operation(
            &client.logical_store_id().0,
            &client.responsibility().0,
            &recovery.operation_id.0,
            "pending",
            None,
            None,
        )
        .await
        .expect("rewind to pending");

    ctx.restart_daemon().await;

    // Post-restart reconciliation maps the atomic message mapping back to an
    // accepted result rather than reporting loss or re-sending.
    match client
        .reconcile_operation(&recovery)
        .await
        .expect("post-restart reconciliation")
    {
        OperationReconciliation::Recorded(record) => match record.outcome {
            RecordedOperationOutcome::Accepted(result)
            | RecordedOperationOutcome::Duplicate(result) => {
                assert_eq!(result.message_id, sent.message_id);
                assert!(result.replayed);
                assert_eq!(result.axes.durable_acceptance, EvidenceState::Accepted);
            }
            other => panic!("expected acceptance from the durable mapping, got {other:?}"),
        },
        other => panic!("expected a recorded operation, got {other:?}"),
    }

    let history = client
        .history(Some(station.address.clone()), false, None, None, None, 50)
        .await
        .expect("history");
    assert_eq!(
        history
            .iter()
            .filter(|item| item.message.id == sent.message_id)
            .count(),
        1,
        "reconciliation must not duplicate the message"
    );
}

// ----------------------------------------------------------------------------------------
// (g) Filtered history and store-scoped source resolution
// ----------------------------------------------------------------------------------------

async fn history_filters_apply_before_bounds_and_require_attachment(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let sender = ctx.spec("hist-sender", ApplicationCapability::SendOnly);
    let station = ctx.spec("hist-station", ApplicationCapability::Bidirectional);
    attach_one(&client, &sender).await;
    attach_one(&client, &station).await;

    let mut sent = Vec::new();
    for index in 0..3 {
        sent.push(
            client
                .send(send_request(
                    &format!("hist-op-{index}"),
                    &sender.address,
                    &station.address,
                    &format!("message {index}"),
                ))
                .await
                .expect("send"),
        );
    }

    // Resolve the *first* message so the unresolved filter and the bound
    // disagree if the bound were applied first.
    let first = client
        .receive(&station.address, Some(2000))
        .await
        .expect("receive")
        .expect("delivery");
    assert_eq!(first.delivery.message_id, sent[0].message_id);
    client
        .disposition(&sender.address, &first.delivery, "handled", None)
        .await
        .expect("resolve the first message");

    let page = client
        .history(Some(station.address.clone()), true, None, None, None, 1)
        .await
        .expect("bounded unresolved history");
    assert_eq!(page.len(), 1);
    assert_eq!(
        page[0].message.id, sent[1].message_id,
        "the unresolved filter must be applied before the bound"
    );
    assert_eq!(&page[0].logical_store_id, client.logical_store_id());

    let unresolved = client
        .history(Some(station.address.clone()), true, None, None, None, 50)
        .await
        .expect("unresolved history");
    assert_eq!(unresolved.len(), 2);
    assert!(unresolved
        .iter()
        .all(|item| item.message.id != sent[0].message_id));

    // after_message_id is a cursor, not a filter substitute.
    let after = client
        .history(
            Some(station.address.clone()),
            false,
            None,
            None,
            Some(sent[0].message_id),
            50,
        )
        .await
        .expect("cursor history");
    assert!(after
        .iter()
        .all(|item| item.message.id > sent[0].message_id));

    // Thread filter: a reply keeps the thread, and the filter honours it.
    let reply = client
        .reply(ReplyRequest {
            operation_id: OperationId("hist-reply".to_string()),
            sender: station.address.clone(),
            message_id: sent[1].message_id,
            cc: Vec::new(),
            kind: "reply".to_string(),
            attention: "background".to_string(),
            requires_disposition: false,
            subject: None,
            body: "reply body".to_string(),
            metadata: None,
            retry_budget: 1,
        })
        .await
        .expect("reply");
    assert_eq!(reply.thread_id, sent[1].thread_id);
    let threaded = client
        .history(
            Some(station.address.clone()),
            false,
            Some(sent[1].thread_id),
            None,
            None,
            50,
        )
        .await
        .expect("thread history");
    assert!(!threaded.is_empty());
    assert!(threaded
        .iter()
        .all(|item| item.message.thread_id == sent[1].thread_id));

    // A recent-only window excludes older traffic.
    let recent = client
        .history(
            Some(station.address.clone()),
            false,
            None,
            Some(now_ms() + 60_000),
            None,
            50,
        )
        .await
        .expect("recent history");
    assert!(recent.is_empty(), "a future since-bound returns nothing");

    // Fail closed on inexact or unbounded requests.
    assert!(matches!(
        client.history(None, false, None, None, None, 10).await,
        Err(ApplicationClientError::InvalidRequest(_))
    ));
    for limit in [0, 1001] {
        assert!(matches!(
            client
                .history(
                    Some(station.address.clone()),
                    false,
                    None,
                    None,
                    None,
                    limit
                )
                .await,
            Err(ApplicationClientError::InvalidRequest(_))
        ));
    }
    assert!(matches!(
        client
            .history(
                Some(ctx.address("never-attached")),
                false,
                None,
                None,
                None,
                10
            )
            .await,
        Err(ApplicationClientError::MembershipLost { .. })
    ));
}

async fn source_resolution_is_store_scoped_and_fails_closed(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let sender = ctx.spec("src-sender", ApplicationCapability::SendOnly);
    let station = ctx.spec("src-station", ApplicationCapability::Bidirectional);
    attach_one(&client, &sender).await;
    attach_one(&client, &station).await;

    let sent = client
        .send(send_request(
            "src-op",
            &sender.address,
            &station.address,
            "source body",
        ))
        .await
        .expect("send");

    let reference = SourceReference {
        logical_store_id: client.logical_store_id().clone(),
        message_id: sent.message_id,
    };
    match client
        .resolve_source(&reference)
        .await
        .expect("resolve in-store source")
    {
        SourceResolution::Authoritative(message) => {
            assert_eq!(message.id, sent.message_id);
            assert_eq!(message.to_addr, station.address);
            assert_eq!(message.body, "source body");
        }
        other => panic!("expected an authoritative source, got {other:?}"),
    }

    // A reference from a different logical store is refused, never resolved
    // against this store's rows.
    let foreign = SourceReference {
        logical_store_id: LogicalStoreId("store-v1-foreign".to_string()),
        message_id: sent.message_id,
    };
    assert!(matches!(
        client
            .resolve_source(&foreign)
            .await
            .expect("resolve foreign"),
        SourceResolution::Mismatch
    ));

    // A missing row is Unavailable, and only an explicit caller capture can
    // downgrade it to CapturedOnly.
    let missing = SourceReference {
        logical_store_id: client.logical_store_id().clone(),
        message_id: sent.message_id + 10_000,
    };
    assert!(matches!(
        client
            .resolve_source(&missing)
            .await
            .expect("resolve missing"),
        SourceResolution::Unavailable
    ));
    let captured = match client
        .resolve_source(&reference)
        .await
        .expect("capture the authoritative row")
    {
        SourceResolution::Authoritative(message) => message,
        other => panic!("expected an authoritative source, got {other:?}"),
    };
    match client
        .resolve_source_with_capture(&missing, Some(captured.clone()))
        .await
        .expect("resolve with capture")
    {
        SourceResolution::CapturedOnly(message) => assert_eq!(message.id, captured.id),
        other => panic!("expected CapturedOnly, got {other:?}"),
    }
    // A foreign store reference still fails closed even with a capture.
    assert!(matches!(
        client
            .resolve_source_with_capture(&foreign, Some(captured))
            .await
            .expect("resolve foreign with capture"),
        SourceResolution::Mismatch
    ));
}

// ----------------------------------------------------------------------------------------
// (h) Ordered deltas, gap detection, resync
// ----------------------------------------------------------------------------------------

async fn delta_pages_are_monotonic_and_gaps_require_resync(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let sender = ctx.spec("delta-sender", ApplicationCapability::SendOnly);
    let station = ctx.spec("delta-station", ApplicationCapability::Bidirectional);
    attach_one(&client, &sender).await;
    attach_one(&client, &station).await;
    for index in 0..4 {
        client
            .send(send_request(
                &format!("delta-op-{index}"),
                &sender.address,
                &station.address,
                &format!("delta {index}"),
            ))
            .await
            .expect("send");
    }

    let page = client.delta_page(0, 100).await.expect("first delta page");
    assert_eq!(&page.logical_store_id, client.logical_store_id());
    assert_eq!(page.from_version, 0);
    assert!(!page.deltas.is_empty());
    assert_eq!(
        page.deltas[0].version, 1,
        "the first page starts at the floor"
    );
    for pair in page.deltas.windows(2) {
        assert_eq!(
            pair[1].version,
            pair[0].version + 1,
            "delta versions are contiguous and monotonic"
        );
    }
    let observed = page.deltas.last().expect("a delta").version;
    assert!(observed <= page.current_version);

    // Following on from the observed version never regresses or repeats.
    client
        .send(send_request(
            "delta-op-tail",
            &sender.address,
            &station.address,
            "tail",
        ))
        .await
        .expect("send tail");
    let next = client
        .delta_page(observed, 100)
        .await
        .expect("follow-on delta page");
    assert_eq!(next.from_version, observed);
    assert!(next.deltas.iter().all(|delta| delta.version > observed));
    assert!(next.current_version >= page.current_version);

    // A version ahead of the store requires a resync rather than a silent gap.
    match client.delta_page(next.current_version + 50, 10).await {
        Err(ApplicationClientError::ResyncRequired {
            observed_version, ..
        }) => assert_eq!(observed_version, next.current_version + 50),
        other => panic!("a future cursor must require resync, got {other:?}"),
    }
    assert!(matches!(
        client.delta_page(0, 0).await,
        Err(ApplicationClientError::InvalidRequest(_))
    ));

    // Prune the retained history and prove a stale follower is told to resync
    // instead of silently skipping the pruned window.
    let maintenance = ctx.maintenance().await;
    assert_eq!(maintenance.logical_store_id(), client.logical_store_id());
    let pruned = maintenance
        .cleanup_deltas(StoreDeltaRetentionPolicy {
            before_version: next.current_version,
            max_delete: 1000,
        })
        .await
        .expect("prune deltas");
    assert!(pruned.deltas_deleted > 0, "cleanup must prune: {pruned:?}");

    match client.delta_page(0, 100).await {
        Err(ApplicationClientError::ResyncRequired {
            expected_version,
            observed_version,
        }) => {
            assert_eq!(observed_version, 0);
            assert!(expected_version > 0);
            // Resync from the advertised floor makes progress with no gap.
            let resynced = client
                .delta_page(expected_version, 100)
                .await
                .expect("resync from the retained floor");
            assert_eq!(resynced.from_version, expected_version);
            assert!(resynced
                .deltas
                .iter()
                .all(|delta| delta.version > expected_version));
            for pair in resynced.deltas.windows(2) {
                assert_eq!(pair[1].version, pair[0].version + 1);
            }
        }
        Ok(page) => panic!(
            "a fully pruned window must require resync, got a page from {} with floor {}",
            page.from_version, page.retained_floor
        ),
        other => panic!("expected ResyncRequired, got {other:?}"),
    }
}

// ----------------------------------------------------------------------------------------
// (i) Compound prerequisite ordering, fencing, partial outcomes, continuation
// ----------------------------------------------------------------------------------------

fn compound_steps() -> Vec<CompoundStep> {
    vec![
        CompoundStep {
            step_id: "reply".to_string(),
            position: 1,
            kind: "reply".to_string(),
            prerequisites: Vec::new(),
            declaration: serde_json::json!({"role": "reply"}),
        },
        CompoundStep {
            step_id: "close".to_string(),
            position: 2,
            kind: "disposition".to_string(),
            prerequisites: vec!["reply".to_string()],
            declaration: serde_json::json!({"role": "terminal"}),
        },
    ]
}

async fn compound_prerequisites_order_and_fence_terminal_steps(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let sender = ctx.spec("compound-sender", ApplicationCapability::SendOnly);
    let station = ctx.spec("compound-station", ApplicationCapability::Bidirectional);
    attach_one(&client, &sender).await;
    attach_one(&client, &station).await;

    let sent = client
        .send(send_request(
            "compound-src",
            &sender.address,
            &station.address,
            "needs a compound answer",
        ))
        .await
        .expect("send");
    let delivery = client
        .receive(&station.address, Some(2000))
        .await
        .expect("receive")
        .expect("delivery");
    assert_eq!(delivery.delivery.message_id, sent.message_id);

    let operation = OperationId("compound-op".to_string());
    let declared = client
        .declare_compound(&operation, &compound_steps())
        .await
        .expect("declare compound steps");
    assert_eq!(declared.len(), 2);
    assert!(declared.iter().all(|step| step.state == "pending"));

    // Declaration validation is fail-closed on missing/self prerequisites.
    assert!(matches!(
        client
            .declare_compound(
                &OperationId("bad-compound".to_string()),
                &[CompoundStep {
                    step_id: "only".to_string(),
                    position: 1,
                    kind: "reply".to_string(),
                    prerequisites: vec!["only".to_string()],
                    declaration: serde_json::json!({}),
                }],
            )
            .await,
        Err(ApplicationClientError::InvalidRequest(_))
    ));

    // A terminal step cannot run before its prerequisite is durably complete.
    let terminal = CompoundDispositionRequest {
        sender: sender.address.clone(),
        delivery: delivery.delivery.clone(),
        state: "handled".to_string(),
        note: Some("compound close".to_string()),
        operation_id: operation.clone(),
        step_id: "close".to_string(),
        outcome: Some(serde_json::json!({"closed": true})),
        recovery: None,
    };
    match client.complete_compound_disposition(&terminal).await {
        Err(ApplicationClientError::Partial(detail)) => {
            assert!(detail.contains("prerequisite"), "{detail}");
        }
        other => panic!("an unmet prerequisite must be Partial, got {other:?}"),
    }
    match client
        .complete_compound_step(&operation, "close", CompoundStepState::Accepted, None, None)
        .await
    {
        Err(ApplicationClientError::Partial(detail)) => {
            assert!(detail.contains("reply"), "{detail}")
        }
        other => panic!("ordering must be enforced, got {other:?}"),
    }

    // Complete the prerequisite, then the fenced terminal step commits.
    let reply = client
        .reply(ReplyRequest {
            operation_id: OperationId("compound-reply".to_string()),
            sender: station.address.clone(),
            message_id: sent.message_id,
            cc: Vec::new(),
            kind: "reply".to_string(),
            attention: "background".to_string(),
            requires_disposition: false,
            subject: None,
            body: "compound reply".to_string(),
            metadata: None,
            retry_budget: 1,
        })
        .await
        .expect("reply");
    assert_eq!(reply.thread_id, sent.thread_id);
    let step = client
        .complete_compound_step(
            &operation,
            "reply",
            CompoundStepState::Accepted,
            Some(&serde_json::json!({"message_id": reply.message_id})),
            None,
        )
        .await
        .expect("complete the prerequisite step");
    assert_eq!(step.state, "accepted");

    let row = client
        .complete_compound_disposition(&terminal)
        .await
        .expect("terminal compound disposition");
    assert_eq!(row.message_id, sent.message_id);
    assert_eq!(row.recipient, station.address);
    assert_eq!(row.state, "handled");

    let steps = client
        .declare_compound(&operation, &compound_steps())
        .await
        .expect("re-declare is idempotent");
    assert_eq!(
        steps
            .iter()
            .find(|step| step.step_id == "close")
            .expect("close step")
            .state,
        "accepted",
        "the terminal step is durably recorded"
    );

    // The terminal step is fenced: a demoted owner cannot re-commit it.
    let backend = ctx.backend().await;
    backend
        .reset_epoch_lease(&station.address)
        .await
        .expect("reset epoch lease");
    backend
        .claim_epoch_lease(&station.address, "successor-owner", 60)
        .await
        .expect("successor claim");
    match client.complete_compound_disposition(&terminal).await {
        Err(ApplicationClientError::MembershipLost {
            reason: MembershipLossReason::OwnerDemoted,
            ..
        }) => {}
        other => panic!("a demoted owner must be fenced, got {other:?}"),
    }
}

async fn compound_partial_outcomes_survive_restart(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let sender = ctx.spec("cpart-sender", ApplicationCapability::SendOnly);
    let station = ctx.spec("cpart-station", ApplicationCapability::Bidirectional);
    attach_one(&client, &sender).await;
    attach_one(&client, &station).await;

    let operation = OperationId("compound-partial".to_string());
    client
        .declare_compound(&operation, &compound_steps())
        .await
        .expect("declare");

    // A partial/indeterminate step outcome carries caller recovery evidence and
    // does not satisfy the prerequisite.
    let partial = client
        .complete_compound_step(
            &operation,
            "reply",
            CompoundStepState::Indeterminate,
            None,
            Some(&serde_json::json!({"resume": "reply"})),
        )
        .await
        .expect("record an indeterminate step outcome");
    assert_eq!(partial.state, "indeterminate");
    assert!(matches!(
        client
            .complete_compound_step(&operation, "close", CompoundStepState::Accepted, None, None)
            .await,
        Err(ApplicationClientError::Partial(_))
    ));

    ctx.restart_daemon().await;
    let outcome = client
        .reconcile_many(
            &[sender.clone(), station.clone()],
            RecoveryPolicy::BoundedRepair { retries: 3 },
        )
        .await;
    assert!(outcome.ready, "reattach after restart: {outcome:?}");

    // Continuation after the restart resumes the declared plan from durable
    // state, without re-declaring or losing the earlier outcome.
    let steps = client
        .declare_compound(&operation, &compound_steps())
        .await
        .expect("re-declare after restart");
    assert_eq!(
        steps
            .iter()
            .find(|step| step.step_id == "reply")
            .expect("reply step")
            .state,
        "indeterminate",
        "the pre-restart outcome survives"
    );
    let resumed = client
        .complete_compound_step(
            &operation,
            "reply",
            CompoundStepState::Accepted,
            Some(&serde_json::json!({"resumed": true})),
            None,
        )
        .await
        .expect("resume the step after restart");
    assert_eq!(resumed.state, "accepted");
    let closed = client
        .complete_compound_step(&operation, "close", CompoundStepState::Accepted, None, None)
        .await
        .expect("complete the fenced step after its prerequisite");
    assert_eq!(closed.state, "accepted");

    assert!(matches!(
        client
            .complete_compound_step(
                &OperationId("unknown-compound".to_string()),
                "missing",
                CompoundStepState::Accepted,
                None,
                None,
            )
            .await,
        Err(ApplicationClientError::InvalidRequest(_))
    ));
}

// ----------------------------------------------------------------------------------------
// (j) Schema migration, bounded cleanup, provenance, public-evidence redaction
// ----------------------------------------------------------------------------------------

async fn schema_migration_and_newer_schema_refusal(ctx: &Ctx<'_>) {
    let migration_store = ctx.leg.prepare_v2_store("actual-v2-migration").await;
    let migrated = ApplicationClient::connect_with_daemon(
        ApplicationClientConfig {
            responsibility: ApplicationResponsibility("station".to_string()),
            backend: Some(migration_store.profile.clone()),
            db_override: None,
        },
        ctx.bootstrap(),
    )
    .await
    .expect("open and migrate the actual v2 store");
    let station = AddressSpec {
        address: "app:schema-v2:recipient".to_string(),
        capability: ApplicationCapability::Bidirectional,
        description: Some("actual v2 migration recipient".to_string()),
        scope: None,
        tags: None,
    };
    attach_one(&migrated, &station).await;
    match migrated
        .resolve_source(&SourceReference {
            logical_store_id: migrated.logical_store_id().clone(),
            message_id: 1,
        })
        .await
        .expect("resolve after migration")
    {
        SourceResolution::Authoritative(message) => {
            assert_eq!(message.id, 1);
            assert_eq!(message.body, "preserved-v2-message");
            assert_eq!(message.to_addr, station.address);
        }
        other => panic!("migration must preserve messages, got {other:?}"),
    }
    let history = migrated
        .history(Some(station.address.clone()), false, None, None, None, 50)
        .await
        .expect("history after migration");
    assert!(history.iter().any(|item| item.message.id == 1));
    ctx.leg.discard(&migration_store).await;

    // A store written by a newer build is refused, fail-closed, before any
    // mutation and before a daemon is involved.
    let future_store = ctx.leg.prepare("schema-future").await;
    ctx.leg.record_future_schema(&future_store).await;
    match ApplicationClient::connect(ApplicationClientConfig {
        responsibility: ApplicationResponsibility("station".to_string()),
        backend: Some(future_store.profile.clone()),
        db_override: None,
    })
    .await
    {
        Err(ApplicationClientError::Unavailable(detail)) => {
            // The refusal is categorised as a schema-version failure and is
            // redacted: it names the category and a stable diagnostic, never
            // the store path, connection string, or raw backend message.
            assert!(
                detail.starts_with("schema-version operation failed"),
                "newer-schema refusal must be categorised: {detail}"
            );
            for marker in ctx.leg.secret_markers(&future_store) {
                assert!(!detail.contains(&marker), "refusal leaked {marker}");
            }
        }
        Err(other) => panic!("a newer schema must be refused as Unavailable, got {other:?}"),
        Ok(_) => panic!("a newer schema must be refused, but connect succeeded"),
    }
    ctx.leg.discard(&future_store).await;
}

async fn bounded_cleanup_retention_generations_and_provenance(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let sender = ctx.spec("clean-sender", ApplicationCapability::SendOnly);
    let station = ctx.spec("clean-station", ApplicationCapability::Bidirectional);
    attach_one(&client, &sender).await;
    attach_one(&client, &station).await;

    for index in 0..3 {
        client
            .send(send_request(
                &format!("clean-op-{index}"),
                &sender.address,
                &station.address,
                &format!("cleanup {index}"),
            ))
            .await
            .expect("send");
    }
    let operation = OperationId("clean-compound".to_string());
    client
        .declare_compound(&operation, &compound_steps())
        .await
        .expect("declare compound steps");

    let before = client.storage_stats().await.expect("storage stats");
    assert!(before.operation_rows >= 3);
    assert!(before.compound_step_rows >= 2);
    assert!(before.oldest_operation_at_ms.is_some());

    // Cleanup is explicitly bounded: it never deletes more than max_delete.
    let bounded = client
        .cleanup(RetentionPolicy {
            completed_before_ms: i64::MAX,
            max_delete: 1,
        })
        .await
        .expect("bounded cleanup");
    assert_eq!(
        bounded.operations_deleted, 1,
        "cleanup must respect its bound"
    );
    let after = client.storage_stats().await.expect("storage stats");
    assert_eq!(after.operation_rows, before.operation_rows - 1);
    assert!(
        after.compound_step_rows >= 2,
        "in-flight compound work is never pruned by an operation sweep"
    );

    // Every prune that removes operation evidence advances the generation, so a
    // handle staged earlier can no longer claim authoritative absence.
    let probe = send_request("clean-probe", &sender.address, &station.address, "probe");
    let staged = client.prepare_send(&probe).await.expect("prepare");
    client
        .cleanup(RetentionPolicy {
            completed_before_ms: i64::MAX,
            max_delete: 100,
        })
        .await
        .expect("second cleanup");
    let restaged = client.prepare_send(&probe).await.expect("re-prepare");
    assert!(
        restaged.retention_generation > staged.retention_generation,
        "pruning operation evidence must advance the retention generation"
    );

    // Principal provenance is reported without carrying credential material.
    let health = client.health().await.expect("health");
    let record = health
        .iter()
        .find(|record| record.address == station.address)
        .expect("station health");
    let provenance = &record.principal;
    assert!(
        matches!(
            provenance.verification,
            PrincipalVerification::Verified
                | PrincipalVerification::Unverified
                | PrincipalVerification::Unavailable
        ),
        "principal verification must be an explicit typed claim"
    );
    let serialized = serde_json::to_string(provenance).expect("serialize provenance");
    for marker in ctx.leg.secret_markers(&ctx.store) {
        assert!(
            !serialized.contains(&marker),
            "principal provenance must not carry connection or credential material"
        );
    }
}

async fn public_evidence_redacts_paths_credentials_and_frames(ctx: &Ctx<'_>) {
    let client = ctx.client("station").await;
    let sender = ctx.spec("redact-sender", ApplicationCapability::SendOnly);
    let station = ctx.spec("redact-station", ApplicationCapability::Bidirectional);
    attach_one(&client, &sender).await;
    attach_one(&client, &station).await;
    let sent = client
        .send(send_request(
            "redact-op",
            &sender.address,
            &station.address,
            "redaction probe",
        ))
        .await
        .expect("send");
    let delivery = client
        .receive(&station.address, Some(2000))
        .await
        .expect("receive")
        .expect("delivery");

    // Force typed failures so their public evidence is included in the sweep.
    let rival = ctx.client("station").await;
    let collision = rival.attach(std::slice::from_ref(&station)).await;
    let unattached = client
        .receive(&ctx.address("never-attached"), Some(50))
        .await;
    let prepared = client
        .prepare_send(&send_request(
            "redact-prepare",
            &sender.address,
            &station.address,
            "handle",
        ))
        .await
        .expect("prepare");

    let mut evidence = String::new();
    evidence.push_str(&serde_json::to_string(&client.health().await.expect("health")).unwrap());
    evidence.push_str(&serde_json::to_string(&collision).unwrap());
    evidence.push_str(&format!("{collision:?}"));
    evidence.push_str(&format!("{unattached:?}"));
    evidence.push_str(&serde_json::to_string(&sent).unwrap());
    evidence.push_str(&serde_json::to_string(&delivery.delivery).unwrap());
    evidence.push_str(&serde_json::to_string(client.logical_store_id()).unwrap());
    evidence.push_str(&serde_json::to_string(&prepared).unwrap());

    // No raw store path, connection string, credential, or store key.
    let mut markers = ctx.leg.secret_markers(&ctx.store);
    markers.push(ctx.store_key());
    for marker in markers {
        assert!(
            !evidence.contains(&marker),
            "public evidence leaked backend connection material: {marker}"
        );
    }
    // No private runtime or install authority paths.
    for path in [
        ctx.iso.run_dir.to_string_lossy().into_owned(),
        ctx.iso.install_root.to_string_lossy().into_owned(),
        ctx.iso.home.to_string_lossy().into_owned(),
    ] {
        assert!(
            !evidence.contains(&path),
            "public evidence leaked a private authority path: {path}"
        );
    }
    // No daemon frame names, backend row/table names, or private storage terms.
    for token in [
        "ApplicationRegister",
        "ApplicationSend",
        "ApplicationAck",
        "HelloAck",
        "StatusReport",
        "admin_cap",
        "singleton",
        "telex_schema_version",
        "application_operations",
        "store_key",
    ] {
        assert!(
            !evidence.contains(token),
            "public evidence leaked a private frame or storage term: {token}"
        );
    }
}

// ----------------------------------------------------------------------------------------
// Shared runner
// ----------------------------------------------------------------------------------------

/// Run every scenario in the battery against one leg, in order, each with its
/// own isolated store. Returns the executed scenario names so the coverage map
/// can be verified as truthful.
async fn run_battery(iso: &Isolation, leg: &dyn Leg, profiles: &Profiles) -> Vec<&'static str> {
    let mut executed: Vec<&'static str> = Vec::new();
    macro_rules! run {
        ($scenario:ident) => {{
            let name = stringify!($scenario);
            executed.push(name);
            eprintln!("[{}] {name}", leg.name());
            let ctx = Ctx::new(iso, leg, profiles, name).await;
            $scenario(&ctx).await;
            ctx.finish().await;
        }};
    }

    run!(identity_is_fresh_stable_and_presentation_independent);
    run!(strict_recovery_refuses_repair_and_bounded_repair_reattaches);
    run!(restart_loses_membership_and_reports_typed_loss);
    run!(deliberate_detach_blocks_strict_recovery);
    run!(predicate_death_ends_receive_with_typed_loss);
    run!(owner_demotion_is_typed_on_disposition);
    run!(collision_evidence_names_owner_and_epoch);
    run!(unknown_membership_reason_projects_without_collapse);
    run!(multi_address_attach_is_atomic_or_compensable);
    run!(compensation_distinguishes_detach_reattach_and_idempotent);
    run!(lifecycle_cancellation_partitions_uncertain_and_untouched);
    run!(crash_continuation_reattaches_after_daemon_restart);
    run!(send_only_membership_has_no_inbound_attendance);
    run!(bidirectional_receive_binds_exact_delivery_and_ack);
    run!(receipt_axes_are_independent_after_durable_acceptance);
    run!(ack_after_durable_ingest_survives_restart);
    run!(operation_replay_is_retry_stable_and_input_sensitive);
    run!(not_recorded_is_exact_and_retention_boundary_is_typed);
    run!(accepted_send_indeterminate_window_exposes_recovery);
    run!(post_restart_reconciliation_maps_pending_operation);
    run!(history_filters_apply_before_bounds_and_require_attachment);
    run!(source_resolution_is_store_scoped_and_fails_closed);
    run!(delta_pages_are_monotonic_and_gaps_require_resync);
    run!(compound_prerequisites_order_and_fence_terminal_steps);
    run!(compound_partial_outcomes_survive_restart);
    run!(schema_migration_and_newer_schema_refusal);
    run!(bounded_cleanup_retention_generations_and_provenance);
    run!(public_evidence_redacts_paths_credentials_and_frames);

    executed
}

// ----------------------------------------------------------------------------------------
// SQLite leg: unique temp root / TELEX_HOME / TELEX_RUN_DIR / TELEX_DB /
// TELEX_INSTALL_ROOT and the absolute branch binary.
// ----------------------------------------------------------------------------------------

#[cfg(feature = "sqlite")]
mod sqlite_leg {
    use super::*;
    use std::path::PathBuf;

    pub struct SqliteLeg {
        pub store_dir: PathBuf,
        pub profiles: Arc<Profiles>,
        pub counter: std::sync::atomic::AtomicU64,
    }

    #[async_trait::async_trait]
    impl Leg for SqliteLeg {
        fn name(&self) -> &'static str {
            "sqlite"
        }

        async fn prepare(&self, scenario: &str) -> CaseStore {
            let index = self
                .counter
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let path = self
                .store_dir
                .join(format!("{scenario}-{index}.db"))
                .to_string_lossy()
                .into_owned();
            let profile_name = format!("case_{index}");
            let mut profile = telex::profiles::implicit_sqlite(Some(&path));
            profile.path = Some(path.clone());
            self.profiles.register(&profile_name, profile);
            let store = CaseStore {
                profile: profile_name,
                sqlite_path: Some(path),
                pg_schema: None,
            };
            // Create the store up front so private induction can open it.
            let _ = self.open_backend(&store).await;
            store
        }

        async fn open_backend(&self, store: &CaseStore) -> Arc<dyn Backend> {
            let path = store.sqlite_path.as_ref().expect("sqlite path");
            let backend =
                telex::backend::sqlite::SqliteBackend::open(path).expect("open sqlite store");
            backend.init_schema().await.expect("init sqlite schema");
            Arc::new(backend)
        }

        async fn prepare_v2_store(&self, scenario: &str) -> CaseStore {
            let index = self
                .counter
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let path = self
                .store_dir
                .join(format!("{scenario}-{index}.db"))
                .to_string_lossy()
                .into_owned();
            let profile_name = format!("case_{index}");
            let mut profile = telex::profiles::implicit_sqlite(Some(&path));
            profile.path = Some(path.clone());
            self.profiles.register(&profile_name, profile);
            let connection =
                rusqlite::Connection::open(&path).expect("create actual SQLite v2 store");
            connection
                .execute_batch(
                    "CREATE TABLE telex_schema_version (
                         singleton INTEGER NOT NULL DEFAULT 1 UNIQUE,
                         version INTEGER NOT NULL
                     );
                     INSERT INTO telex_schema_version(singleton, version) VALUES (1, 2);
                     CREATE TABLE addresses (
                         address TEXT PRIMARY KEY,
                         description TEXT,
                         scope TEXT,
                         tags TEXT,
                         status TEXT NOT NULL DEFAULT 'active',
                         created_at_ms INTEGER NOT NULL
                     );
                     CREATE TABLE leases (
                         address TEXT PRIMARY KEY,
                         occupant TEXT,
                         host TEXT,
                         principal TEXT,
                         description TEXT,
                         tags TEXT,
                         scope TEXT,
                         pid INTEGER,
                         since_ms INTEGER NOT NULL,
                         heartbeat_at_ms INTEGER NOT NULL,
                         lease_epoch INTEGER NOT NULL,
                         owner_instance_id TEXT,
                         daemon_fence_token INTEGER NOT NULL DEFAULT 0
                     );
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
                     CREATE INDEX messages_to_id_idx ON messages(to_addr, id);
                     CREATE INDEX messages_thread_idx ON messages(thread_id, id);
                     CREATE TABLE dispositions (
                         id INTEGER PRIMARY KEY AUTOINCREMENT,
                         message_id INTEGER NOT NULL,
                         recipient TEXT NOT NULL,
                         state TEXT NOT NULL,
                         note TEXT,
                         by_principal TEXT,
                         at_ms INTEGER NOT NULL
                     );
                     CREATE INDEX dispositions_msg_idx ON dispositions(message_id, id);
                     CREATE TABLE deliveries (
                         id INTEGER PRIMARY KEY AUTOINCREMENT,
                         message_id INTEGER NOT NULL,
                         recipient TEXT NOT NULL,
                         occupant TEXT,
                         delivered_at_ms INTEGER NOT NULL,
                         consumed_at_ms INTEGER,
                         UNIQUE(message_id, recipient)
                     );
                     CREATE INDEX deliveries_recipient_pending_idx
                         ON deliveries(recipient, consumed_at_ms, message_id);
                     CREATE TABLE clock_hwm (
                         id INTEGER PRIMARY KEY CHECK (id = 1),
                         hwm_ms INTEGER NOT NULL
                     );
                     INSERT INTO clock_hwm(id, hwm_ms) VALUES (1, 1);
                     CREATE TABLE legacy_cutover_claims (
                         address TEXT PRIMARY KEY,
                         claimed_at_ms INTEGER NOT NULL
                     );
                     CREATE TABLE telex_schema_meta (
                         key TEXT PRIMARY KEY,
                         value TEXT NOT NULL
                     );
                     CREATE TABLE detach_tombstones (
                         session_id TEXT NOT NULL,
                         address TEXT NOT NULL,
                         reason TEXT NOT NULL,
                         at_ms INTEGER NOT NULL,
                         PRIMARY KEY(session_id, address)
                     );
                     CREATE INDEX detach_tombstones_session_idx
                         ON detach_tombstones(session_id);
                     INSERT INTO messages(
                         id, thread_id, from_addr, to_addr, kind, attention,
                         requires_disposition, body, sent_at_ms, created_at_ms
                     ) VALUES (
                         1, 1, 'app:schema-v2:sender', 'app:schema-v2:recipient',
                         'note', 'background', 0, 'preserved-v2-message', 1, 1
                     );",
                )
                .expect("seed actual SQLite v2 store");
            CaseStore {
                profile: profile_name,
                sqlite_path: Some(path),
                pg_schema: None,
            }
        }

        async fn record_future_schema(&self, store: &CaseStore) {
            let path = store.sqlite_path.as_ref().expect("sqlite path");
            let connection = rusqlite::Connection::open(path).expect("open sqlite for induction");
            connection
                .execute("UPDATE telex_schema_version SET version = 999", [])
                .expect("record a newer schema version");
        }

        fn secret_markers(&self, store: &CaseStore) -> Vec<String> {
            let path = store.sqlite_path.clone().expect("sqlite path");
            vec![
                path.clone(),
                format!("sqlite:{path}"),
                self.store_dir.to_string_lossy().into_owned(),
            ]
        }

        async fn discard(&self, store: &CaseStore) {
            if let Some(path) = &store.sqlite_path {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn sqlite_application_client_conformance() {
        let _env = ENV_LOCK.lock().await;
        let iso = Isolation::new("appconf-sqlite");
        let restore = iso.apply_env();

        let store_dir = iso.root.join("stores");
        std::fs::create_dir_all(&store_dir).expect("create store dir");
        let profiles = Arc::new(Profiles::new(iso.config_path.clone()));
        let leg = SqliteLeg {
            store_dir,
            profiles: profiles.clone(),
            counter: std::sync::atomic::AtomicU64::new(0),
        };

        // `Isolation` and `EnvRestore` tear down on unwind, so a panicking
        // scenario still stops the daemon, removes the temp root, and restores
        // the process environment before the failure propagates.
        let executed = run_battery(&iso, &leg, &profiles).await;
        assert_coverage(&executed);
        drop(restore);
    }
}

// ----------------------------------------------------------------------------------------
// Postgres leg: per-run and per-scenario unique schemas, fail-closed under
// TELEX_PG_REQUIRE=1.
// ----------------------------------------------------------------------------------------
#[cfg(feature = "postgres")]
mod postgres_leg {
    use super::*;
    use telex::backend::postgres::{make_tls, sanitize_ident, PgBackend};

    pub struct PostgresLeg {
        pub url: String,
        pub base_schema: String,
        pub profiles: Arc<Profiles>,
        pub counter: std::sync::atomic::AtomicU64,
    }

    impl PostgresLeg {
        fn config(&self) -> tokio_postgres::Config {
            let mut cfg: tokio_postgres::Config = self
                .url
                .parse()
                .expect("TELEX_PG_URL must be a libpq URI or key=value DSN");
            if let Ok(password) = std::env::var("TELEX_PG_PASSWORD") {
                if !password.is_empty() {
                    cfg.password(password);
                }
            }
            cfg
        }

        async fn admin_exec(&self, sql: &str) {
            let cfg = self.config();
            let (client, connection) = cfg
                .connect(make_tls().expect("tls"))
                .await
                .expect("admin connect");
            let handle = tokio::spawn(async move {
                let _ = connection.await;
            });
            let result = client.batch_execute(sql).await;
            drop(client);
            let _ = handle.await;
            result.unwrap_or_else(|e| panic!("admin statement failed: {e}"));
        }

        /// Drop schemas leaked by a previously hard-killed run without ever
        /// touching a live one: match only the exact per-run/per-case shape and
        /// require the embedded creation timestamp to be comfortably old.
        pub async fn sweep_leftover_schemas(&self, base: &str) {
            let cutoff_ms = now_ms() - 3_600_000;
            self.admin_exec(&format!(
                "DO $$ DECLARE s text; ts bigint; BEGIN \
                   FOR s IN SELECT schema_name FROM information_schema.schemata \
                            WHERE schema_name ~ '^{base}_[0-9]+_[0-9]+_[0-9]+$' \
                   LOOP ts := substring(s from '_([0-9]+)_[0-9]+$')::bigint; \
                     IF ts < {cutoff_ms} THEN \
                       EXECUTE format('DROP SCHEMA IF EXISTS %I CASCADE', s); \
                     END IF; \
                   END LOOP; END $$;"
            ))
            .await;
        }
    }

    #[async_trait::async_trait]
    impl Leg for PostgresLeg {
        fn name(&self) -> &'static str {
            "postgres"
        }

        async fn prepare(&self, scenario: &str) -> CaseStore {
            let index = self
                .counter
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let schema = sanitize_ident(&format!("{}_{index}", self.base_schema))
                .expect("derived schema name must be a valid identifier");
            self.admin_exec(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
                .await;
            let profile_name = format!("case_{index}");
            let mut profile = telex::profiles::implicit_sqlite(None);
            profile.kind = "postgres".to_string();
            profile.path = None;
            profile.url = Some(self.url.clone());
            profile.schema = Some(schema.clone());
            profile.auth = Some("password".to_string());
            if std::env::var("TELEX_PG_PASSWORD").is_ok() {
                profile.password_env = Some("TELEX_PG_PASSWORD".to_string());
            }
            let _ = scenario;
            self.profiles.register(&profile_name, profile);
            let store = CaseStore {
                profile: profile_name,
                sqlite_path: None,
                pg_schema: Some(schema),
            };
            let _ = self.open_backend(&store).await;
            store
        }

        async fn open_backend(&self, store: &CaseStore) -> Arc<dyn Backend> {
            let schema = store.pg_schema.as_ref().expect("pg schema");
            let backend = PgBackend::connect_with(self.config(), Some(schema))
                .await
                .expect("connect postgres");
            backend.init_schema().await.expect("init postgres schema");
            Arc::new(backend)
        }

        async fn prepare_v2_store(&self, scenario: &str) -> CaseStore {
            let index = self
                .counter
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let schema = sanitize_ident(&format!("{}_{index}", self.base_schema))
                .expect("derived schema name must be valid");
            self.admin_exec(&format!(
                "DROP SCHEMA IF EXISTS {schema} CASCADE;
                 CREATE SCHEMA {schema};
                 CREATE TABLE {schema}.telex_schema_version (
                     singleton integer NOT NULL DEFAULT 1 UNIQUE,
                     version bigint NOT NULL
                 );
                 INSERT INTO {schema}.telex_schema_version(singleton, version) VALUES (1, 2);
                 CREATE TABLE {schema}.addresses (
                     address text PRIMARY KEY,
                     description text,
                     scope text,
                     tags text,
                     status text NOT NULL DEFAULT 'active',
                     created_at_ms bigint NOT NULL
                 );
                 CREATE TABLE {schema}.leases (
                     address text PRIMARY KEY,
                     occupant text,
                     host text,
                     principal text,
                     description text,
                     tags text,
                     scope text,
                     pid bigint,
                     since_ms bigint NOT NULL,
                     heartbeat_at_ms bigint NOT NULL,
                     lease_epoch bigint,
                     owner_instance_id text,
                     daemon_fence_token bigint NOT NULL DEFAULT 0
                 );
                 CREATE TABLE {schema}.messages (
                     id bigserial PRIMARY KEY,
                     thread_id bigint,
                     parent_id bigint,
                     from_addr text,
                     to_addr text NOT NULL,
                     cc text,
                     kind text NOT NULL DEFAULT 'note',
                     attention text NOT NULL DEFAULT 'background',
                     requires_disposition boolean NOT NULL DEFAULT false,
                     subject text,
                     body text NOT NULL,
                     metadata text,
                     sent_at_ms bigint NOT NULL,
                     created_at_ms bigint NOT NULL
                 );
                 CREATE INDEX messages_to_id_idx ON {schema}.messages(to_addr, id);
                 CREATE INDEX messages_thread_idx ON {schema}.messages(thread_id, id);
                 CREATE TABLE {schema}.dispositions (
                     id bigserial PRIMARY KEY,
                     message_id bigint NOT NULL,
                     recipient text NOT NULL,
                     state text NOT NULL,
                     note text,
                     by_principal text,
                     at_ms bigint NOT NULL
                 );
                 CREATE INDEX dispositions_msg_idx
                     ON {schema}.dispositions(message_id, id);
                 CREATE TABLE {schema}.deliveries (
                     id bigserial PRIMARY KEY,
                     message_id bigint NOT NULL,
                     recipient text NOT NULL,
                     occupant text,
                     delivered_at_ms bigint NOT NULL,
                     consumed_at_ms bigint,
                     UNIQUE(message_id, recipient)
                 );
                 CREATE INDEX deliveries_recipient_pending_idx
                     ON {schema}.deliveries(recipient, consumed_at_ms, message_id);
                 CREATE TABLE {schema}.clock_hwm (
                     id integer PRIMARY KEY CHECK (id = 1),
                     hwm_ms bigint NOT NULL
                 );
                 INSERT INTO {schema}.clock_hwm(id, hwm_ms) VALUES (1, 1);
                 CREATE TABLE {schema}.legacy_cutover_claims (
                     address text PRIMARY KEY,
                     claimed_at_ms bigint NOT NULL
                 );
                 CREATE TABLE {schema}.telex_schema_meta (
                     key text PRIMARY KEY,
                     value text NOT NULL
                 );
                 CREATE TABLE {schema}.detach_tombstones (
                     session_id text NOT NULL,
                     address text NOT NULL,
                     reason text NOT NULL,
                     at_ms bigint NOT NULL,
                     PRIMARY KEY(session_id, address)
                 );
                 CREATE INDEX detach_tombstones_session_idx
                     ON {schema}.detach_tombstones(session_id);
                 INSERT INTO {schema}.messages(
                     id, thread_id, from_addr, to_addr, kind, attention,
                     requires_disposition, body, sent_at_ms, created_at_ms
                 ) VALUES (
                     1, 1, 'app:schema-v2:sender', 'app:schema-v2:recipient',
                     'note', 'background', false, 'preserved-v2-message', 1, 1
                 );"
            ))
            .await;
            let profile_name = format!("case_{index}");
            let mut profile = telex::profiles::implicit_sqlite(None);
            profile.kind = "postgres".to_string();
            profile.path = None;
            profile.url = Some(self.url.clone());
            profile.schema = Some(schema.clone());
            profile.auth = Some("password".to_string());
            if std::env::var("TELEX_PG_PASSWORD").is_ok() {
                profile.password_env = Some("TELEX_PG_PASSWORD".to_string());
            }
            let _ = scenario;
            self.profiles.register(&profile_name, profile);
            CaseStore {
                profile: profile_name,
                sqlite_path: None,
                pg_schema: Some(schema),
            }
        }

        async fn record_future_schema(&self, store: &CaseStore) {
            let schema = store.pg_schema.as_ref().expect("pg schema");
            self.admin_exec(&format!(
                "UPDATE {schema}.telex_schema_version SET version = 999"
            ))
            .await;
        }

        fn secret_markers(&self, store: &CaseStore) -> Vec<String> {
            let mut markers = vec![self.url.clone()];
            if let Some(schema) = &store.pg_schema {
                markers.push(format!("postgres:{}|{schema}", self.url));
            }
            if let Ok(password) = std::env::var("TELEX_PG_PASSWORD") {
                if !password.is_empty() {
                    markers.push(password);
                }
            }
            markers
        }

        async fn discard(&self, store: &CaseStore) {
            if let Some(schema) = &store.pg_schema {
                self.admin_exec(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
                    .await;
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn postgres_application_client_conformance() {
        let _env = ENV_LOCK.lock().await;
        let Some(url) = isolation::postgres_url_or_fail_closed("application-client-conformance")
        else {
            return;
        };
        let iso = Isolation::new("appconf-postgres");
        let restore = iso.apply_env();

        let base = sanitize_ident(
            &std::env::var("TELEX_PG_SCHEMA").unwrap_or_else(|_| "telex_app_conf".into()),
        )
        .expect("TELEX_PG_SCHEMA must be a valid identifier");
        let base_schema = sanitize_ident(&format!("{base}_{}_{}", std::process::id(), now_ms()))
            .expect("derived per-run schema must be a valid identifier");
        let profiles = Arc::new(Profiles::new(iso.config_path.clone()));
        let leg = PostgresLeg {
            url,
            base_schema,
            profiles: profiles.clone(),
            counter: std::sync::atomic::AtomicU64::new(0),
        };
        leg.sweep_leftover_schemas(&base).await;

        let executed = run_battery(&iso, &leg, &profiles).await;
        assert_coverage(&executed);
        drop(restore);
    }
}
