use super::*;
use crate::commands::copilot::session_state::ActivityMarker;
use chrono::SecondsFormat;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "telex-stall-{}-{}-{}",
            std::process::id(),
            crate::model::now_ms(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn member(&self, name: &str, member: &Member) {
        std::fs::write(self.0.join(name), serde_json::to_vec(member).unwrap()).unwrap();
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn at(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(1_793_040_000 + seconds, 0)
        .unwrap()
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parameters() -> Parameters {
    let root = std::env::temp_dir();
    Parameters {
        group_directory: root.join("group"),
        backend_profile: "fixture".into(),
        watcher_executable: root.join("telex-watcher"),
        watcher_registry: root.join("registry.sqlite"),
        grace_seconds: 60,
        poll_interval_seconds: 30,
    }
}

fn member(id: &str) -> Member {
    Member {
        schema_version: 1,
        session_id: id.into(),
        address: format!("member:{id}"),
        state: MemberState::Active,
        reason: None,
    }
}

fn observation(id: &str, state: SessionState) -> Observation {
    Observation {
        member: member(id),
        session: Some(Classification {
            session_id: id.into(),
            state,
            reason: if state == SessionState::Unknown {
                "unrecognized-event"
            } else {
                "fixture"
            },
            activity: Some(ActivityMarker {
                id: format!("activity-{id}"),
                timestamp: at(-120),
            }),
        }),
        wake_sources: Some(WakeSources::default()),
        background_work: "unobserved",
    }
}

fn request(seconds: i64, state: Value) -> DetectorRequest {
    protocol::parse_request(
        &serde_json::to_vec(&json!({
            "schemaVersion":2,
            "attempt":{"id":"fixture-attempt", "now":at(seconds)},
            "watch":{"id":"group-watch","activatedAt":at(-300), "lastSuccessAt":null,
                "parameters":{"group":"fixture"}},
            "state":state,
        }))
        .unwrap(),
    )
    .unwrap()
}

fn run(seconds: i64, state: Value, observations: &[Observation]) -> DetectorResult {
    let result = evaluate(&request(seconds, state), &parameters(), observations).unwrap();
    protocol::validate_result(&result).unwrap();
    result
}

fn commit(result: &DetectorResult, prior: &Value) -> Value {
    result.next_state.clone().unwrap_or_else(|| prior.clone())
}

fn quiet_state(observations: &[Observation]) -> Value {
    let mut state = Value::Null;
    for second in [0, 30, 60] {
        let result = run(second, state.clone(), observations);
        assert_eq!(result.outcome, Outcome::Idle);
        state = commit(&result, &state);
    }
    state
}

#[test]
fn stall_requires_strictly_more_than_grace_and_emits_once() {
    let observations = [observation("a", SessionState::Idle)];
    let state = quiet_state(&observations);
    let event = run(61, state, &observations);
    assert_eq!(event.outcome, Outcome::Event);
    assert_eq!(event.event.as_ref().unwrap().kind, "copilot.group.stalled");
    let body: Value = serde_json::from_str(&event.event.as_ref().unwrap().body).unwrap();
    assert_eq!(body["degraded"], false);
    assert_eq!(body["members"][0]["backgroundWork"], "unobserved");
    let state = event.next_state.unwrap();
    for seconds in [62, 600, 3600] {
        assert_eq!(
            run(seconds, state.clone(), &observations).outcome,
            Outcome::Idle
        );
    }
}

#[test]
fn state_only_commits_after_delivery_so_failed_delivery_reoffers_same_episode() {
    let observations = [observation("a", SessionState::Idle)];
    let prior = quiet_state(&observations);
    let event = run(61, prior.clone(), &observations);
    let retry = run(62, prior, &observations);
    assert_eq!(retry.outcome, Outcome::Event);
    assert_eq!(
        event.next_state,
        retry.next_state.as_ref().map(|state| {
            let mut state = state.clone();
            state["lastObservedAt"] = event.next_state.as_ref().unwrap()["lastObservedAt"].clone();
            state
        })
    );
    assert_eq!(event.event.unwrap().metadata, retry.event.unwrap().metadata);
}

#[test]
fn activity_between_idle_polls_and_membership_changes_restart_episode() {
    let mut observations = vec![observation("a", SessionState::Idle)];
    let state = run(0, Value::Null, &observations).next_state.unwrap();
    observations[0]
        .session
        .as_mut()
        .unwrap()
        .activity
        .as_mut()
        .unwrap()
        .id = "new-stop".into();
    let reset = run(61, state, &observations);
    assert_eq!(reset.outcome, Outcome::Idle);
    let reset_state = reset.next_state.unwrap();
    assert_ne!(reset_state["fingerprint"], Value::Null);
    assert_eq!(
        run(121, reset_state.clone(), &observations).outcome,
        Outcome::Idle
    );
    observations.push(observation("b", SessionState::NotLoaded));
    assert_eq!(run(122, reset_state, &observations).outcome, Outcome::Idle);
}

#[test]
fn activity_after_an_emitted_event_allows_one_new_episode() {
    let mut observations = [observation("a", SessionState::Idle)];
    let initial = quiet_state(&observations);
    let emitted = run(61, initial, &observations).next_state.unwrap();
    observations[0]
        .session
        .as_mut()
        .unwrap()
        .activity
        .as_mut()
        .unwrap()
        .id = "new-id".into();
    let next = run(62, emitted, &observations).next_state.unwrap();
    assert_eq!(next["emitted"], false);
    let next = run(100, next, &observations).next_state.unwrap();
    assert_eq!(run(123, next, &observations).outcome, Outcome::Event);
}

#[test]
fn busy_operator_waits_and_each_observed_wake_source_prevent_stalls() {
    for state in [SessionState::Busy, SessionState::WaitingOnOperator] {
        assert_eq!(
            run(600, Value::Null, &[observation("a", state)]).outcome,
            Outcome::Idle
        );
    }
    for source in ["unacknowledged", "deferred", "watch"] {
        let mut observations = [observation("a", SessionState::Idle)];
        let wakes = observations[0].wake_sources.as_mut().unwrap();
        match source {
            "unacknowledged" => wakes.unacknowledged_messages = 1,
            "deferred" => wakes.deferred_messages = 1,
            _ => wakes
                .armed_watches
                .push("depends-on-another-idle-member".into()),
        }
        assert_eq!(run(600, Value::Null, &observations).outcome, Outcome::Idle);
    }
    let mut observations = [observation("a", SessionState::Idle)];
    observations[0].wake_sources = None;
    assert_eq!(
        run(600, Value::Null, &observations).outcome,
        Outcome::Degraded
    );
}

#[test]
fn inactive_members_do_not_count_and_zero_active_clears_the_episode() {
    assert_eq!(run(0, Value::Null, &[]).next_state, Some(Value::Null));
    for state in [
        MemberState::Parked,
        MemberState::OperatorHeld,
        MemberState::Done,
    ] {
        let mut observations = [observation("a", SessionState::Unknown)];
        observations[0].member.state = state;
        let result = run(600, json!({"ignored":"old state"}), &observations);
        assert_eq!(result.outcome, Outcome::Idle);
        assert_eq!(result.next_state, Some(Value::Null));
    }
}

#[test]
fn quiet_unknowns_emit_degraded_advisory_never_a_stall_and_deduplicate() {
    for reason in [
        "unrecognized-event",
        "session-error",
        "session-started",
        "unsupported-tail",
    ] {
        let mut unknown = observation("unknown", SessionState::Unknown);
        unknown.session.as_mut().unwrap().reason = reason;
        let observations = [observation("idle", SessionState::Idle), unknown];
        let result = run(0, Value::Null, &observations);
        assert_eq!(result.outcome, Outcome::Event);
        assert_eq!(
            result.event.as_ref().unwrap().kind,
            "copilot.group.attention-needed"
        );
        let snapshot: Value = serde_json::from_str(&result.event.as_ref().unwrap().body).unwrap();
        assert_eq!(snapshot["degraded"], true);
        assert_eq!(snapshot["members"][1]["session"]["state"], "unknown");
        assert_eq!(snapshot["members"][1]["session"]["reason"], reason);
        assert_eq!(
            snapshot["members"][1]["session"]["activity"]["timestamp"],
            at(-120)
        );
        let repeated = run(1000, result.next_state.unwrap(), &observations);
        assert_eq!(repeated.outcome, Outcome::Degraded);
        assert!(repeated.next_state.is_none());
        assert!(repeated.event.is_none());
    }
}

#[test]
fn unknown_advisory_requires_old_evidence_and_no_busy_operator_wait_or_wake() {
    let unknown = observation("unknown", SessionState::Unknown);
    for state in [SessionState::Busy, SessionState::WaitingOnOperator] {
        let result = run(
            0,
            Value::Null,
            &[
                observation("other", state),
                observation("unknown", SessionState::Unknown),
            ],
        );
        assert_eq!(result.outcome, Outcome::Degraded);
        assert!(result.next_state.is_none());
    }
    let mut recent = unknown;
    recent
        .session
        .as_mut()
        .unwrap()
        .activity
        .as_mut()
        .unwrap()
        .timestamp = at(0);
    assert_eq!(run(60, Value::Null, &[recent]).outcome, Outcome::Degraded);
    let mut recent_idle = observation("idle", SessionState::Idle);
    recent_idle
        .session
        .as_mut()
        .unwrap()
        .activity
        .as_mut()
        .unwrap()
        .timestamp = at(0);
    let observations = [recent_idle, observation("unknown", SessionState::Unknown)];
    assert_eq!(
        run(60, Value::Null, &observations).outcome,
        Outcome::Degraded
    );
    assert_eq!(run(61, Value::Null, &observations).outcome, Outcome::Event);
    let mut no_timestamp = observation("not-loaded", SessionState::NotLoaded);
    no_timestamp.session.as_mut().unwrap().activity = None;
    assert_eq!(
        run(
            0,
            Value::Null,
            &[no_timestamp, observation("unknown", SessionState::Unknown)]
        )
        .outcome,
        Outcome::Event
    );
    let mut missing = observation("unknown", SessionState::Unknown);
    missing.session.as_mut().unwrap().activity = None;
    assert_eq!(
        run(1000, Value::Null, &[missing]).outcome,
        Outcome::Degraded
    );
    for source in ["unacknowledged", "deferred", "watch"] {
        let mut unknown = observation("unknown", SessionState::Unknown);
        let wakes = unknown.wake_sources.as_mut().unwrap();
        match source {
            "unacknowledged" => wakes.unacknowledged_messages = 1,
            "deferred" => wakes.deferred_messages = 1,
            _ => wakes.armed_watches.push("future-event".into()),
        }
        assert_eq!(
            run(1000, Value::Null, &[unknown]).outcome,
            Outcome::Degraded
        );
    }
}

#[test]
fn unknown_transition_and_advisory_share_one_fingerprint_namespace() {
    let mut observations = [observation("a", SessionState::Unknown)];
    let event = run(0, Value::Null, &observations);
    let emitted = event.next_state.unwrap();
    observations[0].session.as_mut().unwrap().reason = "another-unknown-reason";
    // Changed evidence is a new episode even though it remains unknown.
    assert_eq!(run(61, emitted, &observations).outcome, Outcome::Event);
}

#[test]
fn alternating_degraded_and_successful_observations_cannot_restart_grace_forever() {
    let observations = [observation("a", SessionState::Idle)];
    let mut state = run(0, Value::Null, &observations).next_state.unwrap();
    for seconds in [30, 100] {
        let failed = degraded();
        state = commit(&failed, &state);
        let successful = run(seconds + 40, state.clone(), &observations);
        state = commit(&successful, &state);
        if seconds == 30 {
            assert_eq!(successful.outcome, Outcome::Idle);
            assert_eq!(state["gapRestarted"], true);
            assert_eq!(state["firstSeenAt"], timestamp_ms(&at(70)).unwrap());
        } else {
            assert_eq!(successful.outcome, Outcome::Event);
        }
    }
}

#[test]
fn equal_and_backward_clocks_do_not_emit_prematurely() {
    let observations = [observation("a", SessionState::Idle)];
    let state = run(50, Value::Null, &observations).next_state.unwrap();
    assert_eq!(run(50, state.clone(), &observations).outcome, Outcome::Idle);
    let reset = run(0, state, &observations);
    assert_eq!(reset.outcome, Outcome::Idle);
    assert_eq!(
        reset.next_state.unwrap()["firstSeenAt"],
        timestamp_ms(&at(0)).unwrap()
    );
    for timestamp in ["not-a-time".to_owned(), at(2000)] {
        let mut observations = [observation("a", SessionState::Idle)];
        observations[0]
            .session
            .as_mut()
            .unwrap()
            .activity
            .as_mut()
            .unwrap()
            .timestamp = timestamp;
        assert_eq!(
            run(1000, Value::Null, &observations).outcome,
            Outcome::Degraded
        );
    }
}

#[test]
fn member_records_are_sorted_bounded_and_strict() {
    let directory = Directory::new();
    directory.member("b.json", &member("b"));
    directory.member("a.json", &member("a"));
    let members = read_members(&directory.0).unwrap();
    assert_eq!(
        members
            .iter()
            .map(|member| member.session_id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    directory.member("mismatch.json", &member("c"));
    assert!(read_members(&directory.0).is_err());
    std::fs::remove_file(directory.0.join("mismatch.json")).unwrap();
    let mut duplicate = member("b");
    duplicate.address = "member:a".into();
    directory.member("b.json", &duplicate);
    assert!(read_members(&directory.0).is_err());
    directory.member("b.json", &member("b"));
    std::fs::write(directory.0.join("invalid.json"), b"{").unwrap();
    assert!(read_members(&directory.0).is_err());
    std::fs::write(
        directory.0.join("invalid.json"),
        vec![b'x'; MAX_MEMBER_BYTES + 1],
    )
    .unwrap();
    assert!(read_members(&directory.0).is_err());
    std::fs::write(
        directory.0.join("invalid.json"),
        br#"{"schemaVersion":1,"schemaVersion":1}"#,
    )
    .unwrap();
    assert!(read_members(&directory.0).is_err());
}

#[test]
fn group_and_parameter_bounds_are_enforced() {
    let mut parameters = parameters();
    parameters.validate().unwrap();
    parameters.grace_seconds = 0;
    assert!(parameters.validate().is_err());
    parameters.grace_seconds = 60;
    parameters.group_directory = PathBuf::from("relative");
    assert!(parameters.validate().is_err());
    let directory = Directory::new();
    for index in 0..=MAX_MEMBERS {
        let id = format!("member-{index}");
        directory.member(&format!("{id}.json"), &member(&id));
    }
    assert!(read_members(&directory.0).is_err());
}

#[test]
fn watcher_inventory_requires_current_successful_typed_active_registration_data() {
    let watch = json!({"schemaVersion":2,"id":"other-watch","lifecycle":"active",
        "registration":{"schemaVersion":2,"backendProfile":"fixture","target":"member:a"}});
    let valid = json!({"schemaVersion":1,"command":"list","ok":true,
        "result":{"type":"watches","value":[watch]}});
    let parsed = parse_watches(&serde_json::to_vec(&valid).unwrap()).unwrap();
    assert_eq!(parsed[0].registration.target, "member:a");
    for (pointer, value) in [
        ("/schemaVersion", json!(2)),
        ("/ok", json!(false)),
        ("/command", json!("show")),
        ("/result/type", json!("watch")),
        ("/result/value/0/lifecycle", json!("paused")),
        ("/result/value/0/registration/schemaVersion", json!(1)),
        ("/result/value/0/registration", Value::Null),
    ] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert!(
            parse_watches(&serde_json::to_vec(&invalid).unwrap()).is_err(),
            "{pointer}"
        );
    }
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn read_only_sources_detect_unacknowledged_and_consumed_but_deferred_messages() {
    use crate::backend::sqlite::SqliteBackend;
    use crate::model::NewMessage;
    let directory = Directory::new();
    let path = directory.0.join("store.sqlite");
    let backend = SqliteBackend::open(path.to_str().unwrap()).unwrap();
    backend.init_schema().await.unwrap();
    let message = backend
        .insert_message(&NewMessage {
            to_addr: "member:a".into(),
            body: "fixture".into(),
            requires_disposition: true,
            ..Default::default()
        })
        .await
        .unwrap();
    drop(backend);
    let readonly = SqliteBackend::open_readonly(path.to_str().unwrap()).unwrap();
    let pending = readonly.undelivered_counts().await.unwrap();
    assert_eq!(
        message_wakes(&readonly, &member("a"), &pending)
            .await
            .unwrap()
            .unacknowledged_messages,
        1
    );
    drop(readonly);
    let backend = SqliteBackend::open(path.to_str().unwrap()).unwrap();
    backend
        .mark_delivered(message.id, "member:a", None)
        .await
        .unwrap();
    backend
        .insert_disposition(message.id, "member:a", "deferred", None, None)
        .await
        .unwrap();
    drop(backend);
    let before = std::fs::read(&path).unwrap();
    let readonly = SqliteBackend::open_readonly(path.to_str().unwrap()).unwrap();
    let pending = readonly.undelivered_counts().await.unwrap();
    let wakes = message_wakes(&readonly, &member("a"), &pending)
        .await
        .unwrap();
    assert_eq!(wakes.unacknowledged_messages, 0);
    assert_eq!(wakes.deferred_messages, 1);
    assert!(wakes.present());
    drop(readonly);
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn inbox_suffix_limit_never_hides_older_deferred_work_as_confirmed_absence() {
    use crate::backend::sqlite::SqliteBackend;
    use crate::model::NewMessage;
    let directory = Directory::new();
    let path = directory.0.join("store.sqlite");
    let backend = SqliteBackend::open(path.to_str().unwrap()).unwrap();
    backend.init_schema().await.unwrap();
    for index in 0..=MAX_INBOX_ITEMS + 1 {
        let message = backend
            .insert_message(&NewMessage {
                to_addr: "member:a".into(),
                body: "fixture".into(),
                requires_disposition: true,
                ..Default::default()
            })
            .await
            .unwrap();
        backend
            .mark_delivered(message.id, "member:a", None)
            .await
            .unwrap();
        backend
            .insert_disposition(
                message.id,
                "member:a",
                if index == 0 { "deferred" } else { "handled" },
                None,
                None,
            )
            .await
            .unwrap();
    }
    drop(backend);
    let readonly = SqliteBackend::open_readonly(path.to_str().unwrap()).unwrap();
    let pending = readonly.undelivered_counts().await.unwrap();
    assert!(message_wakes(&readonly, &member("a"), &pending)
        .await
        .is_err());
}
