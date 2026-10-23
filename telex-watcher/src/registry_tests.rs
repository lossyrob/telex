use super::*;
use crate::protocol_v2::parse_registration;
use telex::application_client::*;

struct Fixture {
    root: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("watcher-v2-registry-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        Self {
            root: std::fs::canonicalize(root).unwrap(),
        }
    }
    fn path(&self) -> std::path::PathBuf {
        self.root.join("registry.sqlite")
    }
    fn open(&self) -> Registry {
        Registry::open(&self.path(), 10_000, 10_000_000).unwrap()
    }
    fn registration(&self, id: &str) -> Registration {
        parse_registration(
            &serde_json::to_vec(&json!({
                "schemaVersion":2, "id":id, "command":["detector"],
                "backendProfile":"fixture", "sender":"service:watcher", "target":"agent:recipient",
                "intervalSeconds":1, "timeoutSeconds":10, "initialState":{"cursor":1}
            }))
            .unwrap(),
            &self.root,
        )
        .unwrap()
    }
    fn staged(&self, registry: &mut Registry, id: &str, terminal: bool) -> Operation {
        if registry.get(id).unwrap().is_none() {
            registry.register(self.registration(id), 1000).unwrap();
            registry.bind_store(id, "store-1").unwrap();
        }
        let (watch, attempt) = registry
            .begin_attempt(id, "runtime-original", 2000)
            .unwrap();
        let operation_id = format!("operation-{}", Uuid::new_v4());
        let event_id = format!("event-{}", Uuid::new_v4());
        let sequence = watch.last_allocated_event_sequence + 1;
        let metadata = EventMetadata {
            schema_version: 2,
            watcher: WatcherProvenance {
                watch_id: id.to_owned(),
                event_sequence: sequence,
                event_id: event_id.clone(),
                operation_id: operation_id.clone(),
                attempt_id: attempt.id.clone(),
                runtime_id: attempt.runtime_id.clone(),
                logical_store_id: "store-1".into(),
                registration_revision: 1,
                detector_schema_version: 2,
            },
            detector: Default::default(),
        };
        let request = SendRequest {
            operation_id: OperationId(operation_id.clone()),
            sender: "service:watcher".into(),
            to: "agent:recipient".into(),
            cc: Vec::new(),
            kind: "fixture.condition".into(),
            attention: "next-checkpoint".into(),
            requires_disposition: true,
            subject: Some("condition".into()),
            body: "same content".into(),
            metadata: Some(serde_json::to_string(&metadata).unwrap()),
            retry_budget: 3,
        };
        let recovery = RecoveryHandle {
            logical_store_id: LogicalStoreId("store-1".into()),
            responsibility: ApplicationResponsibility(registry.responsibility.clone()),
            operation_id: request.operation_id.clone(),
            payload_identity: PayloadIdentity {
                algorithm: "sha256".into(),
                digest: "a".repeat(64),
                comparable: true,
            },
            retention_generation: Some(0),
        };
        let op = Operation {
            schema_version: 2,
            watch_id: id.into(),
            event_sequence: sequence,
            event_id,
            operation_id,
            attempt_id: attempt.id,
            attempt_now: attempt.started_at,
            runtime_id: attempt.runtime_id,
            registration_revision: 1,
            registration: watch.registration.unwrap(),
            prior_state_hash: hash_value(&watch.state).unwrap(),
            prior_state: watch.state,
            proposed_state_hash: hash_value(&json!({"cursor":2})).unwrap(),
            proposed_state: json!({"cursor":2}),
            terminal,
            envelope_hash: hash_value(&request).unwrap(),
            envelope: request,
            recovery,
            recurrence_hash: hash_value(&json!({"fixture":"same"})).unwrap(),
            hash_algorithm: "sha256".into(),
            hash_preimage_version: 1,
            status: OperationStatus::Staged,
            query_only: false,
            reconciliation_budget: Budget {
                remaining: 2,
                used: 0,
                deadline_ms: 10_000,
            },
            send_budget: Budget {
                remaining: 2,
                used: 0,
                deadline_ms: 10_000,
            },
            next_recovery_ms: 2000,
            last_evidence_at: None,
            last_evidence: None,
            receipt: None,
        };
        registry.stage(&op, 2100).unwrap();
        op
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).expect("remove isolated registry fixture");
    }
}

fn accepted(op: &Operation) -> SendResult {
    SendResult {
        logical_store_id: op.recovery.logical_store_id.clone(),
        operation_id: op.recovery.operation_id.clone(),
        message_id: 7,
        thread_id: 7,
        sender: op.envelope.sender.clone(),
        recipient: op.envelope.to.clone(),
        axes: ReceiptAxes {
            durable_acceptance: EvidenceState::Accepted,
            occupied_at_acceptance: Some(false),
            push_acceptance: EvidenceState::NotAttempted,
            recipient_consumption: EvidenceState::Unknown,
            workflow_disposition: EvidenceState::Unknown,
        },
        payload_identity: op.recovery.payload_identity.clone(),
        replayed: false,
    }
}

fn not_recorded(op: &Operation) -> SendEvidence {
    SendEvidence::Reconciliation(Ok(OperationReconciliation::NotRecorded(
        NotRecordedEvidence {
            logical_store_id: op.recovery.logical_store_id.clone(),
            responsibility: op.recovery.responsibility.clone(),
            operation_id: op.recovery.operation_id.clone(),
            payload_identity: op.recovery.payload_identity.clone(),
            retention_generation: 0,
        },
    )))
}

fn rejected(permanent: bool) -> SendEvidence {
    SendEvidence::send(Err(ApplicationClientError::RejectedBeforeAcceptance {
        code: "fixture-rejection".into(),
        detail: "fixture".into(),
        retryability: if permanent {
            RejectionRetryability::Permanent
        } else {
            RejectionRetryability::Transient
        },
    }))
}

#[test]
fn physical_registry_lock_rejects_same_path_and_hard_link_without_mutation() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    registry
        .register(fixture.registration("watch"), 1000)
        .unwrap();
    let before = std::fs::read(fixture.path()).unwrap();
    assert!(Registry::open(&fixture.path(), 1, 1)
        .err()
        .unwrap()
        .to_string()
        .contains("registry-already-owned"));
    let alias = fixture.root.join("alias.sqlite");
    std::fs::hard_link(fixture.path(), &alias).unwrap();
    assert!(Registry::open(&alias, 1, 1).is_err());
    assert_eq!(before, std::fs::read(fixture.path()).unwrap());
    drop(registry);
    let reopened = fixture.open();
    assert_eq!(reopened.list().unwrap().len(), 1);
}

#[test]
fn staging_is_durable_and_fences_detector_and_all_unsafe_mutations() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    let op = fixture.staged(&mut registry, "watch", false);
    crate::staging::verify(&op).unwrap();
    let watch = registry.get("watch").unwrap().unwrap();
    assert_eq!(watch.state, json!({"cursor":1}));
    assert_eq!(watch.last_allocated_event_sequence, 1);
    assert_eq!(watch.last_committed_event_sequence, 0);
    assert_eq!(watch.eligibility, Eligibility::ReconciliationPending);
    assert!(registry.begin_attempt("watch", "runtime", 3000).is_err());
    for change in [
        Change::Pause,
        Change::Resume,
        Change::Update {
            registration: Box::new(fixture.registration("watch")),
            state: Some(Value::Null),
        },
    ] {
        assert_eq!(
            registry.change("watch", change, 3000).unwrap().refusal,
            Some(Refusal::PendingOperation)
        );
    }
    drop(registry);
    let mut registry = fixture.open();
    registry.start_runtime("restarted", 3000, |_| true).unwrap();
    let restored = registry.operation(&op.operation_id).unwrap().unwrap();
    assert_eq!(restored.envelope, op.envelope);
    assert_eq!(restored.recovery, op.recovery);
    assert_eq!(restored.proposed_state, op.proposed_state);
    assert_eq!(restored.status, OperationStatus::QueryRequired);
    assert!(!registry
        .reserve_recovery(&op.operation_id, true, false, 3000)
        .unwrap());
}

#[test]
fn durable_acceptance_alone_commits_even_without_attendance_push_or_consumption() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    let op = fixture.staged(&mut registry, "watch", false);
    registry
        .evidence(
            &op.operation_id,
            SendEvidence::send(Ok(accepted(&op))),
            3000,
        )
        .unwrap();
    let watch = registry.get("watch").unwrap().unwrap();
    assert_eq!(watch.state, json!({"cursor":2}));
    assert_eq!(watch.last_committed_event_sequence, 1);
    assert_eq!(watch.last_success_at, Some(op.attempt_now.clone()));
    assert_eq!(watch.health, Health::Ready);
    let repeat = fixture.staged(&mut registry, "watch", false);
    assert_eq!(repeat.event_sequence, 2);
    assert_ne!(repeat.event_id, op.event_id);
    registry
        .evidence(
            &repeat.operation_id,
            SendEvidence::send(Ok(accepted(&repeat))),
            4000,
        )
        .unwrap();
    assert_eq!(
        registry
            .get("watch")
            .unwrap()
            .unwrap()
            .recurrence
            .consecutive_identical_recurrences,
        2
    );
}

#[test]
fn acceptance_commit_failure_preserves_original_operation_for_restart_reconciliation() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    let op = fixture.staged(&mut registry, "watch", true);
    registry.fail_next_accept_commit = true;
    assert!(registry
        .evidence(
            &op.operation_id,
            SendEvidence::send(Ok(accepted(&op))),
            3000
        )
        .is_err());
    assert_eq!(
        registry.get("watch").unwrap().unwrap().state,
        json!({"cursor":1})
    );
    drop(registry);
    let mut registry = fixture.open();
    registry.start_runtime("next", 4000, |_| true).unwrap();
    let record = RecordedOperation {
        logical_store_id: op.recovery.logical_store_id.clone(),
        responsibility: op.recovery.responsibility.clone(),
        operation_id: op.recovery.operation_id.clone(),
        sender: op.envelope.sender.clone(),
        target: OperationTarget::Send {
            to: op.envelope.to.clone(),
            cc: Vec::new(),
        },
        payload_identity: op.recovery.payload_identity.clone(),
        retry_budget: op.envelope.retry_budget,
        outcome: RecordedOperationOutcome::Duplicate(accepted(&op)),
    };
    registry
        .evidence(
            &op.operation_id,
            SendEvidence::Reconciliation(Ok(OperationReconciliation::Recorded(Box::new(record)))),
            5000,
        )
        .unwrap();
    let watch = registry.get("watch").unwrap().unwrap();
    assert_eq!(watch.lifecycle, Lifecycle::Terminal);
    assert_eq!(watch.state, json!({"cursor":2}));
}

#[test]
fn unknown_and_retention_boundary_never_permit_resend_or_state_commit() {
    for evidence in [
        SendEvidence::ObservationCancelled,
        SendEvidence::send(Err(ApplicationClientError::Unavailable(
            "history unavailable".into(),
        ))),
        SendEvidence::Reconciliation(Ok(OperationReconciliation::RetentionBoundaryCrossed {
            staged_generation: Some(0),
            current_generation: 1,
        })),
    ] {
        let fixture = Fixture::new();
        let mut registry = fixture.open();
        let op = fixture.staged(&mut registry, "watch", false);
        registry.evidence(&op.operation_id, evidence, 3000).unwrap();
        assert!(!registry
            .reserve_recovery(&op.operation_id, true, false, 4000)
            .unwrap());
        assert!(registry
            .reserve_recovery(&op.operation_id, false, false, 4000)
            .unwrap());
        registry
            .evidence(&op.operation_id, SendEvidence::ObservationCancelled, 5000)
            .unwrap();
        assert!(registry
            .reserve_recovery(&op.operation_id, false, false, 7000)
            .unwrap());
        registry
            .evidence(&op.operation_id, SendEvidence::ObservationCancelled, 8000)
            .unwrap();
        let watch = registry.get("watch").unwrap().unwrap();
        assert_eq!(
            watch.blocked_reason,
            Some(BlockedReason::ReceiptOutcomeUnresolved)
        );
        assert_eq!(watch.state, json!({"cursor":1}));
    }
}

#[test]
fn late_proof_requires_separate_finite_send_grant_and_preserves_activation() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    let op = fixture.staged(&mut registry, "watch", false);
    registry
        .evidence(&op.operation_id, SendEvidence::ObservationCancelled, 11_000)
        .unwrap();
    let before = registry.get("watch").unwrap().unwrap();
    assert_eq!(
        registry
            .change(
                "watch",
                Change::GrantSendRetry {
                    attempts: 1,
                    seconds: 10
                },
                12_000
            )
            .unwrap()
            .refusal,
        Some(Refusal::WrongRecoveryProof)
    );
    assert!(registry
        .change(
            "watch",
            Change::GrantReconciliation {
                attempts: 1,
                seconds: 10
            },
            12_000
        )
        .unwrap()
        .refusal
        .is_none());
    registry
        .evidence(&op.operation_id, not_recorded(&op), 13_000)
        .unwrap();
    let proved = registry.get("watch").unwrap().unwrap();
    assert_eq!(proved.lifecycle, Lifecycle::Paused);
    assert_eq!(
        proved.blocked_reason,
        Some(BlockedReason::SendRetryExhausted)
    );
    assert!(!registry
        .reserve_recovery(&op.operation_id, true, false, 13_000)
        .unwrap());
    assert!(registry
        .change(
            "watch",
            Change::GrantSendRetry {
                attempts: 1,
                seconds: 10
            },
            14_000
        )
        .unwrap()
        .refusal
        .is_none());
    assert!(registry
        .reserve_recovery(&op.operation_id, true, false, 14_000)
        .unwrap());
    let retry = registry.operation(&op.operation_id).unwrap().unwrap();
    assert_eq!(retry.envelope, op.envelope);
    assert_eq!(retry.recovery, op.recovery);
    assert_eq!(
        registry.get("watch").unwrap().unwrap().activated_at,
        before.activated_at
    );
    registry
        .evidence(&op.operation_id, rejected(false), 15_000)
        .unwrap();
    assert_eq!(
        registry.get("watch").unwrap().unwrap().blocked_reason,
        Some(BlockedReason::SendRetryExhausted)
    );
}

#[test]
fn permanent_rejection_requires_audited_close_and_never_reuses_sequence() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    let op = fixture.staged(&mut registry, "watch", false);
    registry
        .evidence(&op.operation_id, rejected(true), 3000)
        .unwrap();
    assert_eq!(
        registry.get("watch").unwrap().unwrap().blocked_reason,
        Some(BlockedReason::SendPermanentlyRejected)
    );
    assert_eq!(
        registry
            .change(
                "watch",
                Change::GrantSendRetry {
                    attempts: 1,
                    seconds: 1
                },
                4000
            )
            .unwrap()
            .refusal,
        Some(Refusal::WrongRecoveryProof)
    );
    assert!(registry
        .change("watch", Change::CloseNotAccepted, 4000)
        .unwrap()
        .refusal
        .is_none());
    assert!(registry
        .change("watch", Change::Resume, 5000)
        .unwrap()
        .refusal
        .is_none());
    let next = fixture.staged(&mut registry, "watch", false);
    assert_eq!(next.event_sequence, 2);
    assert_eq!(registry.events("watch", 100).unwrap().len(), 2);
    assert_eq!(
        registry.get("watch").unwrap().unwrap().state,
        json!({"cursor":1})
    );
}

#[test]
fn mismatched_store_operation_payload_and_nonaccepted_receipts_fail_closed() {
    for field in [
        "store",
        "operation",
        "payload",
        "sender",
        "recipient",
        "acceptance",
    ] {
        let fixture = Fixture::new();
        let mut registry = fixture.open();
        let op = fixture.staged(&mut registry, "watch", false);
        let mut receipt = accepted(&op);
        match field {
            "store" => receipt.logical_store_id = LogicalStoreId("another".into()),
            "operation" => receipt.operation_id = OperationId("another".into()),
            "payload" => receipt.payload_identity.digest = "b".repeat(64),
            "sender" => receipt.sender = "another".into(),
            "recipient" => receipt.recipient = "another".into(),
            "acceptance" => receipt.axes.durable_acceptance = EvidenceState::Unknown,
            _ => unreachable!(),
        }
        registry
            .evidence(&op.operation_id, SendEvidence::send(Ok(receipt)), 3000)
            .unwrap();
        let watch = registry.get("watch").unwrap().unwrap();
        assert_eq!(watch.state, json!({"cursor":1}));
        assert_eq!(watch.last_committed_event_sequence, 0);
        assert_eq!(
            watch.blocked_reason,
            match field {
                "store" => Some(BlockedReason::LogicalStoreMismatch),
                "acceptance" => None,
                _ => Some(BlockedReason::OperationIdentityConflict),
            }
        );
    }
}

#[test]
fn removed_pending_operations_retain_identity_close_without_resurrection_and_never_reuse_ids() {
    for accept in [false, true] {
        let fixture = Fixture::new();
        let mut registry = fixture.open();
        let op = fixture.staged(&mut registry, "watch", true);
        registry.change("watch", Change::Remove, 3000).unwrap();
        assert!(!registry
            .reserve_recovery(&op.operation_id, true, false, 4000)
            .unwrap());
        assert!(registry
            .reserve_recovery(&op.operation_id, false, true, 4000)
            .unwrap());
        registry
            .evidence(
                &op.operation_id,
                if accept {
                    SendEvidence::send(Ok(accepted(&op)))
                } else {
                    not_recorded(&op)
                },
                5000,
            )
            .unwrap();
        let watch = registry.get("watch").unwrap().unwrap();
        assert_eq!(watch.lifecycle, Lifecycle::Removed);
        assert!(watch.pending_operation_id.is_none());
        assert_eq!(
            registry
                .register(fixture.registration("watch"), 6000)
                .unwrap()
                .refusal,
            Some(Refusal::IdAlreadyAllocated)
        );
        assert_eq!(
            registry.events("watch", 10).unwrap()[0].envelope,
            op.envelope
        );
    }
}

#[test]
fn interrupted_attempts_require_containment_proof_and_do_not_allocate_events() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    registry
        .register(fixture.registration("watch"), 1000)
        .unwrap();
    let (_, attempt) = registry.begin_attempt("watch", "old", 2000).unwrap();
    registry
        .containment(&attempt.id, json!({"fixture":"unproven"}))
        .unwrap();
    registry.start_runtime("new", 3000, |_| false).unwrap();
    let watch = registry.get("watch").unwrap().unwrap();
    assert_eq!(
        watch.blocked_reason,
        Some(BlockedReason::OrphanContainmentUnproven)
    );
    assert_eq!(watch.last_allocated_event_sequence, 0);
    assert_eq!(watch.state, json!({"cursor":1}));
    assert_eq!(
        registry
            .change("watch", Change::Resume, 4000)
            .unwrap()
            .refusal,
        Some(Refusal::BlockedReasonUncorrected)
    );
    registry.recheck_containment("watch", |_| true).unwrap();
    assert!(registry
        .change("watch", Change::Resume, 5000)
        .unwrap()
        .refusal
        .is_none());
    assert_eq!(
        registry.attempts("watch", 10).unwrap()[0].outcome,
        AttemptOutcome::Interrupted
    );
}

#[test]
fn explicit_null_state_update_is_distinct_and_inflight_old_revision_cannot_commit() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    registry
        .register(fixture.registration("watch"), 1000)
        .unwrap();
    let (_, attempt) = registry.begin_attempt("watch", "old", 2000).unwrap();
    let changed = registry
        .change(
            "watch",
            Change::Update {
                registration: Box::new(fixture.registration("watch")),
                state: Some(Value::Null),
            },
            3000,
        )
        .unwrap();
    assert_eq!(changed.watch.unwrap().registration_revision, 2);
    assert!(!registry.attempt_is_current(&attempt).unwrap());
    assert!(registry
        .finish_direct(&attempt.id, json!({"cursor":9}), false, 4000)
        .is_err());
    assert_eq!(registry.get("watch").unwrap().unwrap().state, Value::Null);
    registry
        .fail_attempt(
            &attempt.id,
            DiagnosticCategory::RuntimeInterrupted,
            "changed",
            4000,
        )
        .unwrap();
    registry
        .change(
            "watch",
            Change::Update {
                registration: Box::new(fixture.registration("watch")),
                state: None,
            },
            5000,
        )
        .unwrap();
    assert_eq!(registry.get("watch").unwrap().unwrap().state, Value::Null);
}

#[test]
fn v1_rows_are_preserved_but_never_eligible_or_reusable() {
    let fixture = Fixture::new();
    {
        let connection = Connection::open(fixture.path()).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE watches(id TEXT PRIMARY KEY,state_json TEXT NOT NULL);
            INSERT INTO watches VALUES('old-watch','{\"cursor\":5}');",
            )
            .unwrap();
    }
    let mut registry = fixture.open();
    let old = registry.get("old-watch").unwrap().unwrap();
    assert_eq!(old.schema_version, 1);
    assert_eq!(
        old.blocked_reason,
        Some(BlockedReason::UnsupportedRegistrationVersion)
    );
    assert!(registry
        .begin_attempt("old-watch", "runtime", 1000)
        .is_err());
    assert_eq!(
        registry
            .register(fixture.registration("old-watch"), 1000)
            .unwrap()
            .refusal,
        Some(Refusal::IdAlreadyAllocated)
    );
    registry
        .register(fixture.registration("new-watch"), 1000)
        .unwrap();
    assert_eq!(registry.list().unwrap().len(), 2);
}

#[test]
fn legacy_removal_retains_opaque_state_tombstones_and_explicit_audit() {
    for raw in [
        r#"{"cursor":5}"#,
        r#"{"checksum":18446744073709551615}"#,
        r#"{"nested":[-9223372036854775808]}"#,
        r#"{"opaque":"\uffff"}"#,
    ] {
        let fixture = Fixture::new();
        let connection = Connection::open(fixture.path()).unwrap();
        connection
            .execute_batch("CREATE TABLE watches(id TEXT PRIMARY KEY,state_json TEXT NOT NULL);")
            .unwrap();
        connection
            .execute("INSERT INTO watches VALUES('old-watch',?1)", [raw])
            .unwrap();
        drop(connection);

        let mut registry = fixture.open();
        let mut expected = registry.get("old-watch").unwrap().unwrap();
        assert_eq!(expected.state, serde_json::from_str::<Value>(raw).unwrap());
        for change in [
            Change::Pause,
            Change::Resume,
            Change::Update {
                registration: Box::new(fixture.registration("old-watch")),
                state: None,
            },
            Change::GrantReconciliation {
                attempts: 1,
                seconds: 1,
            },
            Change::GrantSendRetry {
                attempts: 1,
                seconds: 1,
            },
            Change::CloseNotAccepted,
        ] {
            assert_eq!(
                registry.change("old-watch", change, 1000).unwrap().refusal,
                Some(Refusal::UnsupportedRegistrationVersion)
            );
        }
        let before_revision = registry.revision().unwrap();
        let removed = registry.change("old-watch", Change::Remove, 2000).unwrap();
        assert!(removed.refusal.is_none());
        expected.lifecycle = Lifecycle::Removed;
        expected.eligibility = Eligibility::Inactive;
        expected.health = Health::Inactive;
        expected.blocked_reason = None;
        assert_eq!(
            serde_json::to_value(removed.watch.unwrap()).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        assert_eq!(registry.revision().unwrap(), before_revision + 1);
        drop(registry);

        let mut registry = fixture.open();
        assert_eq!(
            serde_json::to_value(registry.get("old-watch").unwrap().unwrap()).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        assert_eq!(
            registry
                .register(fixture.registration("old-watch"), 3000)
                .unwrap()
                .refusal,
            Some(Refusal::IdAlreadyAllocated)
        );
        assert!(registry
            .begin_attempt("old-watch", "runtime", 3000)
            .is_err());
        assert_eq!(
            registry
                .change("old-watch", Change::Remove, 3000)
                .unwrap()
                .refusal,
            Some(Refusal::InvalidLifecycle)
        );
        let retained: String = registry
            .connection
            .query_row(
                "SELECT state_json FROM watches WHERE id='old-watch'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained, raw);
        let audits: Vec<Value> = read_many(
            &registry.connection,
            "SELECT data FROM watcher_v2_audits WHERE watch_id='old-watch' AND action='remove' ORDER BY id",
            [],
        )
        .unwrap();
        assert_eq!(
            audits,
            vec![
                json!({
                    "revision": 1,
                    "stateChanged": false,
                    "stateHashStatus": "not-applicable",
                    "stateHashReason": "unsupported-registration-version"
                }),
                json!({"refusal": "invalid-lifecycle"})
            ]
        );
        let isolation: Vec<Value> = read_many(
            &registry.connection,
            "SELECT data FROM watcher_v2_audits WHERE watch_id='old-watch' AND action='isolate-v1'",
            [],
        )
        .unwrap();
        assert_eq!(isolation, vec![json!({"version": 1})]);
    }
}

#[test]
fn legacy_removal_rolls_back_when_audit_or_registry_revision_fails() {
    for failure in [
        "CREATE TRIGGER fail_removal BEFORE INSERT ON watcher_v2_audits
         WHEN NEW.action='remove' BEGIN SELECT RAISE(ABORT,'audit-failed'); END;",
        "CREATE TRIGGER fail_removal BEFORE UPDATE OF revision ON watcher_v2_meta
         BEGIN SELECT RAISE(ABORT,'revision-failed'); END;",
    ] {
        let fixture = Fixture::new();
        let raw = r#"{"checksum":18446744073709551615}"#;
        let connection = Connection::open(fixture.path()).unwrap();
        connection
            .execute_batch("CREATE TABLE watches(id TEXT PRIMARY KEY,state_json TEXT NOT NULL);")
            .unwrap();
        connection
            .execute("INSERT INTO watches VALUES('old-watch',?1)", [raw])
            .unwrap();
        drop(connection);
        let mut registry = fixture.open();
        let mut original = registry.get("old-watch").unwrap().unwrap();
        original.registration_revision = 7;
        save_watch(&registry.connection, &original).unwrap();
        let before_revision = registry.revision().unwrap();
        let before_audits: Vec<Value> = read_many(
            &registry.connection,
            "SELECT data FROM watcher_v2_audits ORDER BY id",
            [],
        )
        .unwrap();
        registry.connection.execute_batch(failure).unwrap();
        let error = registry
            .change("old-watch", Change::Remove, 2000)
            .unwrap_err()
            .to_string();
        assert!(error.contains(if failure.contains("audit-failed") {
            "audit-failed"
        } else {
            "revision-failed"
        }));
        drop(registry);

        let mut registry = fixture.open();
        assert_eq!(
            serde_json::to_value(registry.get("old-watch").unwrap().unwrap()).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
        assert_eq!(registry.revision().unwrap(), before_revision);
        let audits: Vec<Value> = read_many(
            &registry.connection,
            "SELECT data FROM watcher_v2_audits ORDER BY id",
            [],
        )
        .unwrap();
        assert_eq!(audits, before_audits);
        let retained: String = registry
            .connection
            .query_row(
                "SELECT state_json FROM watches WHERE id='old-watch'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained, raw);
        registry
            .connection
            .execute_batch("DROP TRIGGER fail_removal")
            .unwrap();
        let removed = registry.change("old-watch", Change::Remove, 3000).unwrap();
        assert!(removed.refusal.is_none());
        assert_eq!(removed.watch.unwrap().lifecycle, Lifecycle::Removed);
        assert_eq!(registry.revision().unwrap(), before_revision + 1);
        let audits: Vec<Value> = read_many(
            &registry.connection,
            "SELECT data FROM watcher_v2_audits WHERE action='remove'",
            [],
        )
        .unwrap();
        assert_eq!(
            audits,
            vec![json!({
                "revision": 7,
                "stateChanged": false,
                "stateHashStatus": "not-applicable",
                "stateHashReason": "unsupported-registration-version"
            })]
        );
    }
}

#[test]
fn v2_mutations_require_valid_hashes_and_keep_existing_audit_shape() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    registry
        .register(fixture.registration("watch"), 1000)
        .unwrap();
    let original = registry.get("watch").unwrap().unwrap();
    for state in [
        json!(u64::MAX),
        json!({"nested": [i64::MIN]}),
        json!("\u{ffff}"),
    ] {
        assert!(registry
            .change(
                "watch",
                Change::Update {
                    registration: Box::new(fixture.registration("watch")),
                    state: Some(state.clone()),
                },
                2000
            )
            .unwrap_err()
            .to_string()
            .contains("canonicalization-failed"));
        assert_eq!(
            serde_json::to_value(registry.get("watch").unwrap().unwrap()).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
        let mut invalid = original.clone();
        invalid.state = state;
        save_watch(&registry.connection, &invalid).unwrap();
        assert!(registry
            .change("watch", Change::Remove, 2000)
            .unwrap_err()
            .to_string()
            .contains("canonicalization-failed"));
        assert_eq!(
            serde_json::to_value(registry.get("watch").unwrap().unwrap()).unwrap(),
            serde_json::to_value(&invalid).unwrap()
        );
        save_watch(&registry.connection, &original).unwrap();
        assert_eq!(registry.revision().unwrap(), 1);
    }
    registry
        .change(
            "watch",
            Change::Update {
                registration: Box::new(fixture.registration("watch")),
                state: Some(Value::Null),
            },
            3000,
        )
        .unwrap();
    registry.change("watch", Change::Remove, 4000).unwrap();
    let audits: Vec<Value> = read_many(
        &registry.connection,
        "SELECT data FROM watcher_v2_audits WHERE watch_id='watch' AND action IN ('update','remove') ORDER BY id",
        [],
    ).unwrap();
    assert_eq!(
        audits,
        vec![
            json!({
                "revision": 2,
                "oldStateHash": hash_value(&original.state).unwrap(),
                "newStateHash": hash_value(&Value::Null).unwrap()
            }),
            json!({
                "revision": 2,
                "oldStateHash": hash_value(&Value::Null).unwrap(),
                "newStateHash": hash_value(&Value::Null).unwrap()
            }),
        ]
    );
}

#[test]
fn cadence_coalesces_missed_ticks_and_retention_has_positive_warning_thresholds() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    registry
        .register(fixture.registration("watch"), 1000)
        .unwrap();
    let (_, attempt) = registry.begin_attempt("watch", "runtime", 2000).unwrap();
    registry.coalesce(9000).unwrap();
    let watch = registry.get("watch").unwrap().unwrap();
    assert!(watch.suppressed_due_count >= 5);
    assert!(watch.coalesced_due);
    registry
        .finish_direct(&attempt.id, watch.state, false, 9000)
        .unwrap();
    assert!(registry.get("watch").unwrap().unwrap().coalesced_due);
    let retention = registry.retention(Some("watch")).unwrap();
    assert!(retention.rows > 0 && retention.bytes > 0);
    assert!(retention.warning_rows > 0 && retention.warning_bytes > 0);
}

#[test]
fn pause_then_resume_cannot_accept_a_cancelled_attempt_even_at_the_same_wall_clock() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    registry
        .register(fixture.registration("watch"), 1000)
        .unwrap();
    let (_, attempt) = registry.begin_attempt("watch", "runtime", 1000).unwrap();
    registry.change("watch", Change::Pause, 1000).unwrap();
    registry.change("watch", Change::Resume, 1000).unwrap();
    assert!(!registry.attempt_is_current(&attempt).unwrap());
    assert!(registry
        .finish_direct(&attempt.id, Value::Null, false, 1000)
        .is_err());
    assert!(registry.begin_attempt("watch", "runtime", 1000).is_err());
}

#[test]
fn clock_adjustment_preserves_attempt_time_and_resume_preserves_last_success() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    registry
        .register(fixture.registration("watch"), 3000)
        .unwrap();
    let (_, attempt) = registry.begin_attempt("watch", "runtime", 2000).unwrap();
    registry
        .finish_direct(&attempt.id, Value::Null, false, 1000)
        .unwrap();
    let watch = registry.get("watch").unwrap().unwrap();
    assert_eq!(watch.last_success_at, Some(timestamp(2000)));
    assert_eq!(watch.activated_at, timestamp(3000));
    registry.change("watch", Change::Pause, 1000).unwrap();
    registry.change("watch", Change::Resume, 500).unwrap();
    let watch = registry.get("watch").unwrap().unwrap();
    assert_eq!(watch.activated_at, timestamp(500));
    assert_eq!(watch.last_success_at, Some(timestamp(2000)));
}

#[test]
fn removed_transient_rejection_is_retained_as_rejection_not_absence() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    let op = fixture.staged(&mut registry, "watch", false);
    registry.change("watch", Change::Remove, 3000).unwrap();
    registry
        .evidence(&op.operation_id, rejected(false), 4000)
        .unwrap();
    assert_eq!(
        registry
            .operation(&op.operation_id)
            .unwrap()
            .unwrap()
            .status,
        OperationStatus::RemovedRejected
    );
    assert_eq!(
        registry
            .receipt_evidence(&op.operation_id, 10)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn recovery_deadline_expires_even_when_sender_is_unavailable() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    let op = fixture.staged(&mut registry, "watch", false);
    assert!(!registry.expire_recovery(&op.operation_id, 11000).unwrap());
    assert_eq!(
        registry.get("watch").unwrap().unwrap().blocked_reason,
        Some(BlockedReason::SendRetryExhausted)
    );
    assert_eq!(
        registry
            .operation(&op.operation_id)
            .unwrap()
            .unwrap()
            .send_budget
            .used,
        0
    );
}

#[test]
fn a_future_registry_version_is_refused_before_schema_mutation() {
    let fixture = Fixture::new();
    let connection = Connection::open(fixture.path()).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE watcher_v2_meta(singleton INTEGER PRIMARY KEY, version INTEGER);
        INSERT INTO watcher_v2_meta VALUES(1,3);",
        )
        .unwrap();
    drop(connection);
    let before = std::fs::read(fixture.path()).unwrap();
    assert!(Registry::open(&fixture.path(), 1, 1)
        .err()
        .unwrap()
        .to_string()
        .contains("unsupported-registry-version"));
    assert_eq!(before, std::fs::read(fixture.path()).unwrap());
}

#[test]
fn restoring_the_original_store_allows_only_explicit_audited_resume() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    registry
        .register(fixture.registration("watch"), 1000)
        .unwrap();
    assert!(registry.bind_store("watch", "store-a").unwrap());
    assert!(!registry.bind_store("watch", "store-b").unwrap());
    assert_eq!(
        registry
            .change("watch", Change::Resume, 2000)
            .unwrap()
            .refusal,
        Some(Refusal::BlockedReasonUncorrected)
    );
    assert!(registry.bind_store("watch", "store-a").unwrap());
    let restored = registry.get("watch").unwrap().unwrap();
    assert_eq!(restored.lifecycle, Lifecycle::Paused);
    assert_eq!(restored.health, Health::Inactive);
    assert_eq!(restored.blocked_reason, None);
    assert_eq!(restored.activated_at, timestamp(1000));
    assert_eq!(restored.logical_store_id.as_deref(), Some("store-a"));
    assert_eq!(
        restored.observed_logical_store_id.as_deref(),
        Some("store-a")
    );
    let observations: u64 = registry
        .connection
        .query_row(
            "SELECT count(*) FROM watcher_v2_audits WHERE action='store-binding-observed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(observations, 3);
    let resumed = registry.change("watch", Change::Resume, 3000).unwrap();
    assert_eq!(resumed.refusal, None);
    assert_eq!(resumed.watch.unwrap().lifecycle, Lifecycle::Active);
}

#[test]
fn store_observations_cannot_demote_terminal_or_removed_watches() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    registry
        .register(fixture.registration("watch"), 1000)
        .unwrap();
    registry.bind_store("watch", "store-a").unwrap();
    let (_, attempt) = registry.begin_attempt("watch", "runtime", 2000).unwrap();
    registry
        .finish_direct(&attempt.id, Value::Null, true, 3000)
        .unwrap();
    assert!(!registry.bind_store("watch", "store-b").unwrap());
    let terminal = registry.get("watch").unwrap().unwrap();
    assert_eq!(terminal.lifecycle, Lifecycle::Terminal);
    assert_eq!(terminal.eligibility, Eligibility::Inactive);
    assert_eq!(terminal.health, Health::Inactive);
    assert_eq!(terminal.blocked_reason, None);
    registry.change("watch", Change::Remove, 4000).unwrap();
    registry.bind_store("watch", "store-b").unwrap();
    assert_eq!(
        registry.get("watch").unwrap().unwrap().lifecycle,
        Lifecycle::Removed
    );
}

#[test]
fn containment_and_store_fences_must_both_clear_before_resume() {
    for restore_store_first in [true, false] {
        let fixture = Fixture::new();
        let mut registry = fixture.open();
        registry
            .register(fixture.registration("watch"), 1000)
            .unwrap();
        registry.bind_store("watch", "store-a").unwrap();
        let (_, attempt) = registry.begin_attempt("watch", "old", 2000).unwrap();
        registry
            .containment(&attempt.id, json!({"fixture":"unproven"}))
            .unwrap();
        registry.start_runtime("new", 3000, |_| false).unwrap();
        registry.bind_store("watch", "store-b").unwrap();
        assert_eq!(
            registry.get("watch").unwrap().unwrap().blocked_reason,
            Some(BlockedReason::OrphanContainmentUnproven)
        );
        assert_eq!(
            registry
                .change("watch", Change::Resume, 4000)
                .unwrap()
                .refusal,
            Some(Refusal::BlockedReasonUncorrected)
        );
        if restore_store_first {
            registry.bind_store("watch", "store-a").unwrap();
            assert_eq!(
                registry.get("watch").unwrap().unwrap().blocked_reason,
                Some(BlockedReason::OrphanContainmentUnproven)
            );
        }
        registry.recheck_containment("watch", |_| true).unwrap();
        if !restore_store_first {
            assert_eq!(
                registry.get("watch").unwrap().unwrap().blocked_reason,
                Some(BlockedReason::LogicalStoreMismatch)
            );
            assert_eq!(
                registry
                    .change("watch", Change::Resume, 4000)
                    .unwrap()
                    .refusal,
                Some(Refusal::BlockedReasonUncorrected)
            );
            registry.bind_store("watch", "store-a").unwrap();
        }
        assert_eq!(
            registry.get("watch").unwrap().unwrap().lifecycle,
            Lifecycle::Paused
        );
        assert_eq!(
            registry
                .change("watch", Change::Resume, 5000)
                .unwrap()
                .refusal,
            None
        );
    }
}

#[test]
fn store_observations_preserve_pending_recovery_and_rejection_fences() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    let operation = fixture.staged(&mut registry, "watch", false);
    registry
        .evidence(
            &operation.operation_id,
            SendEvidence::ObservationCancelled,
            11000,
        )
        .unwrap();
    registry.bind_store("watch", "wrong-store").unwrap();
    assert_eq!(
        registry.get("watch").unwrap().unwrap().blocked_reason,
        Some(BlockedReason::ReceiptOutcomeUnresolved)
    );
    registry.bind_store("watch", "store-1").unwrap();
    let restored = registry.get("watch").unwrap().unwrap();
    assert_eq!(
        restored.blocked_reason,
        Some(BlockedReason::ReceiptOutcomeUnresolved)
    );
    assert_eq!(
        restored.pending_operation_id.as_deref(),
        Some(operation.operation_id.as_str())
    );
    assert_eq!(
        registry
            .change("watch", Change::Resume, 12000)
            .unwrap()
            .refusal,
        Some(Refusal::PendingOperation)
    );
    assert_eq!(
        registry
            .change(
                "watch",
                Change::GrantReconciliation {
                    attempts: 1,
                    seconds: 10
                },
                12000
            )
            .unwrap()
            .refusal,
        None
    );
}

#[test]
fn restoring_a_pending_store_mismatch_does_not_bypass_receipt_recovery() {
    let fixture = Fixture::new();
    let mut registry = fixture.open();
    let operation = fixture.staged(&mut registry, "watch", false);
    registry.bind_store("watch", "wrong-store").unwrap();
    registry.bind_store("watch", "store-1").unwrap();
    assert_eq!(
        registry.get("watch").unwrap().unwrap().blocked_reason,
        Some(BlockedReason::LogicalStoreMismatch)
    );
    registry
        .evidence(&operation.operation_id, not_recorded(&operation), 3000)
        .unwrap();
    assert_eq!(
        registry.get("watch").unwrap().unwrap().blocked_reason,
        Some(BlockedReason::SendRetryExhausted)
    );
}
