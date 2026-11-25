//! External scheduler enrollment and single-scan receive. Every mutation holds delivery admission.
use super::*;
use crate::scheduled::{self as records, Action, Binding, Phase, Reservation};
use serde_json::{json, Value};

fn outcome(value: Value) -> Response {
    Response::Scheduled { result: value }
}

pub(super) fn require_unscheduled(
    state: &DaemonState,
    store: &str,
    session: &str,
    address: &str,
) -> std::result::Result<(), Response> {
    match records::load(&state.paths, session) {
        Ok(Some(b)) if b.matches(store, address) => Err(proto::incompatible(
            "this station has a scheduled binding; explicitly unbind it first",
        )),
        Err(e) => Err(proto::internal(e.to_string())),
        _ => Ok(()),
    }
}

fn require_no_push_intent(
    state: &DaemonState,
    store: &str,
    session: &str,
    address: &str,
) -> std::result::Result<(), Response> {
    use crate::station_intent::IntentId;
    let Some(scope) = state.intent_store_readonly().map_err(proto::internal)? else {
        return Ok(());
    };
    let id = IntentId::derive(store, session, address);
    if crate::platform_fs::path_present(&scope.path_for(&id))
        .map_err(|e| proto::internal(e.to_string()))?
    {
        let intent = scope
            .load(&id)
            .map_err(|e| proto::internal(e.to_string()))?;
        if intent.state != proto::IntentRecoveryState::Revoked {
            return Err(proto::incompatible(
                "stop the existing push station before scheduling it",
            ));
        }
    }
    Ok(())
}

fn project(state: &DaemonState, b: &Binding) {
    if let Some(member) = state
        .members
        .lock()
        .unwrap()
        .get_mut(&DaemonState::member_key(
            &b.store_key,
            &b.session,
            &b.address,
        ))
    {
        member.scheduled_check = (b.phase == Phase::Active).then(|| b.observation.clone());
    }
}

fn summary(b: &Binding) -> Response {
    outcome(
        json!({"result":"binding","binding":b.generation,"session":b.session,"address":b.address,
        "phase":b.phase,"automation":b.automation,"backend":b.selector,"interval_seconds":b.observation.interval_seconds,
        "scheduled_check":b.observation,"next_due_ms":b.observation.due_ms(),"health":if b.phase==Phase::Active {Some(b.observation.health(now_ms()))}else{None}}),
    )
}

pub(super) async fn request(
    state: Arc<DaemonState>,
    store: String,
    session: String,
    address: String,
    generation: Option<String>,
    action: Action,
) -> Response {
    if session.is_empty()
        || session.len() > 256
        || address.is_empty()
        || address.len() > 1024
        || store.len() > 8192
    {
        return proto::incompatible("invalid scheduled station target");
    }
    if state.is_draining() {
        return proto::error_response(proto::ERROR_NOT_RUNNING, "daemon is draining");
    }
    let _guard = match state
        .admit_binding(&store, &session, &address, reconcile::TEARDOWN_DEADLINE)
        .await
    {
        Ok(guard) => guard,
        Err(e) => return proto::internal(e),
    };
    let existing = match records::load(&state.paths, &session) {
        Ok(b) => b,
        Err(e) => return proto::internal(e.to_string()),
    };
    if let Action::Prepare {
        interval_seconds,
        selector,
    } = action
    {
        if selector.as_ref().is_some_and(|s| s.len() > 256) {
            return proto::incompatible("backend name too long");
        }
        let backend = match state.backend_for(&store).await {
            Ok(b) => b,
            Err(e) => return e,
        };
        let logical_id = match backend.logical_store_id().await {
            Ok(id) => id,
            Err(e) => return proto::internal(e.to_string()),
        };
        if let Some(b) = existing.as_ref().filter(|b| b.phase != Phase::Disabled) {
            if b.matches(&store, &address)
                && b.observation.interval_seconds == interval_seconds
                && b.selector == selector
                && b.logical_store_id == logical_id
            {
                return summary(b);
            }
            return proto::incompatible("this session already has a different scheduled binding");
        }
        if state.get_member(&store, &session, &address).is_some()
            || state.has_live_waiter_for(&store, &session, &address)
        {
            return proto::incompatible("stop the existing station before scheduling it");
        }
        if let Err(response) = require_no_push_intent(&state, &store, &session, &address) {
            return response;
        }
        let binding = match Binding::prepare(
            store,
            logical_id,
            selector,
            session,
            address,
            interval_seconds,
        ) {
            Ok(b) => b,
            Err(e) => return proto::incompatible(e.to_string()),
        };
        // Different addresses use different admissions. Serialize creation for a common session.
        let _lock = match records::prepare_lock(&state.paths, &binding.session) {
            Ok(l) => l,
            Err(e) => return proto::internal(e.to_string()),
        };
        match records::load(&state.paths, &binding.session) {
            Ok(Some(b)) if b.phase != Phase::Disabled => {
                return proto::incompatible("session already enrolled")
            }
            Err(e) => return proto::internal(e.to_string()),
            _ => {}
        }
        return match binding.save(&state.paths) {
            Ok(()) => summary(&binding),
            Err(e) => proto::internal(e.to_string()),
        };
    }
    let Some(mut b) = existing.filter(|b| {
        Some(&b.generation) == generation.as_ref() && b.store_key == store && b.address == address
    }) else {
        return outcome(json!({"result":"inactive"}));
    };
    if matches!(action, Action::Status) {
        return summary(&b);
    }
    if matches!(action, Action::Disable) {
        if b.phase == Phase::Disabled
            && state
                .get_member(&store, &session, &address)
                .and_then(|m| m.scheduled_check)
                .is_none()
        {
            return summary(&b);
        }
        let release_local = state.get_member(&store, &session, &address).is_some();
        b.phase = Phase::Disabled;
        b.reservation = None;
        if let Err(e) = b.save(&state.paths) {
            return proto::internal(e.to_string());
        }
        if release_local {
            let response = detach_member_admitted(state.clone(), store, session, address).await;
            if !matches!(response, Response::Ack { .. }) {
                return response;
            }
        }
        return summary(&b);
    }
    if b.phase == Phase::Disabled
        || (b.phase == Phase::Pending && !matches!(action, Action::Confirm { .. }))
    {
        return outcome(json!({"result":"inactive"}));
    }
    if let Action::Confirm { automation } = &action {
        if automation.trim().is_empty() || automation.len() > 256 {
            return proto::incompatible("invalid automation identifier");
        }
        if b.automation.as_ref().is_some_and(|a| a != automation) {
            return proto::incompatible("binding belongs to a different automation");
        }
    }
    let checking = matches!(action, Action::Check { .. });
    if checking {
        b.observation.last_attempt_ms = Some(now_ms())
    }
    let response = if checking {
        match tokio::time::timeout(Duration::from_secs(10), run(&state, &mut b, action)).await {
            Ok(response) => response,
            Err(_) => proto::error_response(
                "CheckTimeout",
                "scheduled operation exceeded its I/O deadline",
            ),
        }
    } else {
        run(&state, &mut b, action).await
    };
    if checking {
        match &response {
            Response::Error { .. } => b.observation.last_outcome = Some("failed".into()),
            Response::DeliveryQuarantined { .. } => {
                b.observation.last_outcome = Some("quarantined".into())
            }
            Response::Scheduled { result } => {
                b.observation.last_outcome = result["result"].as_str().map(String::from)
            }
            _ => {}
        }
    }
    if let Err(e) = b.save(&state.paths) {
        return proto::internal(e.to_string());
    }
    project(&state, &b);
    response
}

async fn run(state: &Arc<DaemonState>, b: &mut Binding, action: Action) -> Response {
    // A daemon can still have a live connection to an unlinked SQLite file. Verify the store
    // currently at the bound path before a tick can open a replacement or stamp attendance.
    #[cfg(feature = "sqlite")]
    if let Some(path) = b.store_key.strip_prefix("sqlite:") {
        let reopened = match SqliteBackend::open_readonly(path) {
            Ok(db) => db,
            Err(e) => return proto::internal(format!("opening bound store: {e}")),
        };
        match reopened.logical_store_id().await {
            Ok(id) if id == b.logical_store_id => {}
            Ok(_) => return proto::incompatible("logical store changed; bind explicitly"),
            Err(e) => return proto::internal(format!("reading bound store identity: {e}")),
        }
    }
    let backend = match state.backend_for(&b.store_key).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    match backend.logical_store_id().await {
        Ok(id) if id == b.logical_store_id => {}
        Ok(_) => return proto::incompatible("logical store changed; bind explicitly"),
        Err(e) => return proto::internal(e.to_string()),
    }
    let member = state.get_member(&b.store_key, &b.session, &b.address);
    if member.as_ref().is_some_and(|m| {
        m.idle || m.on_deliver.is_some() || m.capability == StationCapability::SendOnly
    }) {
        return proto::incompatible("station is idle or has conflicting delivery coverage");
    }
    if member.is_none() {
        let explicit = b.phase == Phase::Pending && matches!(action, Action::Confirm { .. });
        // An intentional detach is sticky. Only the explicit setup/confirm can clear it.
        if !explicit {
            if !matches!(action, Action::Check { .. } | Action::Confirm { .. }) {
                return proto::incompatible(
                    "membership lost; check again before settling delivery",
                );
            }
            match backend.get_lease(&b.address).await {
                Ok(Some(row)) if row.lease_epoch == b.lease_epoch => {}
                Ok(_) => return proto::incompatible("station lease changed; explicitly rebind"),
                Err(e) => return proto::internal(e.to_string()),
            }
            match backend.detach_tombstone(&b.session, &b.address).await {
                Ok(None) => {}
                Ok(Some(_)) => return outcome(json!({"result":"inactive"})),
                Err(e) => return proto::internal(e.to_string()),
            }
        }
        if let Err(response) = require_no_push_intent(state, &b.store_key, &b.session, &b.address) {
            return response;
        }
        let attached = register_member_admitted(
            state.clone(),
            b.store_key.clone(),
            b.address.clone(),
            b.session.clone(),
            b.session.clone(),
            None,
            None,
            None,
            vec![],
            false,
            !explicit,
            None,
            false,
            false,
            None,
            None,
        )
        .await;
        match attached {
            Response::Registered { lease_epoch, .. } => b.lease_epoch = Some(lease_epoch),
            other => return other,
        }
    }
    let Some(m) = state.get_member(&b.store_key, &b.session, &b.address) else {
        return proto::incompatible("membership lost during scheduled check");
    };
    if b.lease_epoch != Some(m.lease_epoch) {
        return proto::incompatible("different membership generation; explicitly rebind");
    }
    match backend
        .heartbeat_epoch(&b.address, &m.owner_instance_id, m.lease_epoch)
        .await
    {
        Ok(true) => {}
        Ok(false) => return proto::incompatible("station lease has another owner"),
        Err(e) => return proto::internal(e.to_string()),
    }
    match action {
        Action::Confirm { automation } => {
            b.automation = Some(automation);
            b.phase = Phase::Active;
            b.observation.confirmed_at_ms.get_or_insert_with(now_ms);
            summary(b)
        }
        Action::Check { receipt } => scan(state, b, &m, &backend, receipt).await,
        Action::Ack { receipt } if b.last_ack.as_ref() == Some(&receipt) => {
            outcome(json!({"result":"acknowledged"}))
        }
        Action::Ack { receipt } | Action::Reply { receipt, .. }
            if !b.reservation.as_ref().is_some_and(|r| {
                r.receipt == receipt && r.epoch == m.lease_epoch && now_ms() < r.expires_ms
            }) =>
        {
            proto::error_response("StaleReceipt", "check again for a current delivery receipt")
        }
        Action::Ack { receipt } => {
            let r = b.reservation.as_ref().expect("validated receipt");
            let response = ack_exact_delivery(
                state.clone(),
                b.store_key.clone(),
                b.session.clone(),
                b.address.clone(),
                r.message_id,
                r.delivery_id,
            )
            .await;
            match response {
                Response::Ack {
                    delivery_outcome:
                        Some(DeliveryOutcome::Marked | DeliveryOutcome::AlreadyConsumed),
                    ..
                } => {
                    b.last_ack = Some(receipt);
                    b.reservation = None;
                    outcome(json!({"result":"acknowledged"}))
                }
                Response::Ack { .. } => proto::incompatible("delivery could not be consumed"),
                other => other,
            }
        }
        Action::Reply { body, .. } => reply(state, b, &backend, body).await,
        _ => proto::incompatible("invalid scheduled operation"),
    }
}

async fn scan(
    state: &Arc<DaemonState>,
    b: &mut Binding,
    m: &MemberRecord,
    backend: &Arc<dyn Backend>,
    receipt: String,
) -> Response {
    if receipt.len() != 32 || !receipt.bytes().all(|c| c.is_ascii_hexdigit()) {
        return proto::incompatible("invalid receipt nonce");
    }
    let now = now_ms();
    if b.reservation
        .as_ref()
        .is_some_and(|r| r.epoch == m.lease_epoch && now < r.expires_ms)
    {
        return outcome(json!({"result":"busy"}));
    }
    b.reservation = None;
    let candidate = match backend.first_pending_delivery(&b.address).await {
        Ok(v) => v,
        Err(e) => return proto::internal(e.to_string()),
    };
    // Selection can cross an epoch transition, including when it returns an empty set.
    match backend
        .heartbeat_epoch(&b.address, &m.owner_instance_id, m.lease_epoch)
        .await
    {
        Ok(true) => {}
        Ok(false) => return proto::incompatible("lost lease during scan"),
        Err(e) => return proto::internal(e.to_string()),
    }
    if state.is_draining() {
        return proto::error_response(proto::ERROR_NOT_RUNNING, "daemon draining");
    }
    let now = now_ms();
    let Some(c) = candidate else {
        b.observation.last_success_ms = Some(now);
        b.observation.last_outcome = Some("empty".into());
        return outcome(json!({"result":"empty"}));
    };
    let Some(delivery_id) = c.delivery_id else {
        return proto::internal("missing exact delivery row");
    };
    let id = c.message.id;
    let frame = delivery_frame(
        c.message,
        &b.address,
        delivery_id,
        c.snapshot_version,
        m.lease_epoch,
    );
    let bytes = match proto::json_line_frame_len(&frame) {
        Ok(n) => n,
        Err(e) => return proto::internal(e.to_string()),
    };
    if bytes > proto::MAX_JSONL_FRAME_BYTES {
        let note = format!(
            "daemon rejected delivery frame: serialized_bytes={bytes}; max_bytes={}",
            proto::MAX_JSONL_FRAME_BYTES
        );
        return match backend
            .application_disposition_with_ack(
                &b.address,
                &m.owner_instance_id,
                m.lease_epoch,
                id,
                delivery_id,
                "rejected",
                Some(&note),
                Some("daemon"),
                Some("daemon-quarantine"),
                None,
            )
            .await
        {
            Ok((Some(_), DeliveryOutcome::Marked | DeliveryOutcome::AlreadyConsumed)) => {
                Response::DeliveryQuarantined {
                    message_id: id,
                    recipient: b.address.clone(),
                    serialized_bytes: bytes,
                    max_bytes: proto::MAX_JSONL_FRAME_BYTES,
                    may_continue: true,
                }
            }
            Ok(_) => proto::incompatible("lost scheduled quarantine ownership"),
            Err(e) => proto::internal(e.to_string()),
        };
    }
    b.reservation = Some(Reservation {
        receipt,
        message_id: id,
        delivery_id,
        epoch: m.lease_epoch,
        expires_ms: now + i64::from(b.observation.interval_seconds) * 1000,
    });
    b.observation.last_success_ms = Some(now);
    b.observation.last_outcome = Some("delivery".into());
    frame
}

async fn reply(
    state: &Arc<DaemonState>,
    b: &Binding,
    backend: &Arc<dyn Backend>,
    body: String,
) -> Response {
    use crate::model::{ApplicationOperationBegin, NewApplicationOperation};
    let r = b.reservation.as_ref().expect("validated receipt");
    let responsibility = format!("scheduled:{}", b.address);
    let op = format!("reply-{}-{}", r.message_id, r.delivery_id);
    let fingerprint = crate::platform_fs::sha256_hex(body.as_bytes());
    let record = NewApplicationOperation {
        logical_store_id: b.logical_store_id.clone(),
        application_responsibility: responsibility.clone(),
        operation_id: op.clone(),
        operation_kind: "reply".into(),
        sender: b.address.clone(),
        recipients_json: json!([r.message_id, []]).to_string(),
        payload_fingerprint: fingerprint.clone(),
        retry_budget: 0,
        created_at_ms: now_ms(),
    };
    match backend.begin_application_operation(&record).await {
        Ok(ApplicationOperationBegin::FingerprintMismatch(_)) => {
            return proto::incompatible("a different reply was already staged for this delivery")
        }
        Ok(ApplicationOperationBegin::Replay(existing)) => {
            if existing.state == "accepted" {
                return match existing.result_json.as_deref().map(serde_json::from_str) {
                    Some(Ok(receipt)) => Response::Sent { receipt },
                    _ => proto::internal("missing accepted reply receipt"),
                };
            }
            if existing.state != "pending" {
                return proto::incompatible("inspect the indeterminate reply before retrying");
            }
            // A crash can leave the atomic mapping committed before its receipt is recorded.
            match backend
                .application_operation_message(&b.logical_store_id, &responsibility, &op)
                .await
            {
                Ok(Some(row)) => {
                    let receipt = SentReceipt {
                        receipt: "accepted".into(),
                        id: row.id,
                        thread_id: row.thread_id,
                        parent_id: row.parent_id,
                        to: row.to_addr,
                        from: row.from_addr,
                        attention: None,
                        requires_disposition: None,
                        occupied: None,
                    };
                    return finish_reply(backend, &record, receipt).await;
                }
                Ok(None) => {}
                Err(e) => return proto::internal(e.to_string()),
            }
        }
        Ok(ApplicationOperationBegin::Started(_)) => {}
        Err(e) => return proto::internal(e.to_string()),
    }
    // Both backends atomically map the operation to the accepted message and reuse it on replay.
    match reply_message(
        state.clone(),
        b.store_key.clone(),
        b.session.clone(),
        Some(b.address.clone()),
        r.message_id,
        "note".into(),
        "background".into(),
        false,
        None,
        None,
        body,
        None,
        Some((b.logical_store_id.clone(), responsibility, op, fingerprint)),
    )
    .await
    {
        Response::Sent { receipt } => finish_reply(backend, &record, receipt).await,
        response => response,
    }
}

async fn finish_reply(
    backend: &Arc<dyn Backend>,
    record: &crate::model::NewApplicationOperation,
    receipt: SentReceipt,
) -> Response {
    let result = match serde_json::to_string(&receipt) {
        Ok(result) => result,
        Err(error) => return proto::internal(error.to_string()),
    };
    match backend
        .complete_application_operation(
            &record.logical_store_id,
            &record.application_responsibility,
            &record.operation_id,
            "accepted",
            Some(&result),
            None,
        )
        .await
    {
        Ok(saved) if saved.state == "accepted" => Response::Sent { receipt },
        Ok(_) => proto::internal("reply recorded with an unexpected operation state"),
        Err(e) => proto::internal(e.to_string()),
    }
}
