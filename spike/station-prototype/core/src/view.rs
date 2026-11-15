//! Derives the UI snapshot from the durable local projection.

use crate::card;
use crate::model::*;
use crate::org;
use std::collections::BTreeMap;

pub fn short_address(address: &str) -> String {
    address
        .rsplit(['/', ':'])
        .find(|s| !s.is_empty())
        .unwrap_or(address)
        .to_string()
}

fn one_line(text: &str, max: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        let cut: String = flat.chars().take(max).collect();
        format!("{cut}…")
    } else {
        flat
    }
}

fn message_view(m: &StoredMessage, read_through: i64) -> MessageView {
    let parsed = card::parse(m.metadata.as_deref());
    MessageView {
        id: m.id,
        local_id: None,
        direction: m.direction,
        from: m.from.clone(),
        to: m.to.clone(),
        cc: m.cc.clone(),
        kind: m.kind.clone(),
        attention: m.attention.clone(),
        tier: m.tier(),
        requires_disposition: m.requires_disposition,
        subject: m.subject.clone(),
        body: m.body.clone(),
        sent_at_ms: m.sent_at_ms,
        role: m.role.clone(),
        disposition: m.disposition.clone(),
        open_obligation: m.open_obligation(),
        unread: m.direction == Direction::Inbound && m.id > read_through,
        card: if m.direction == Direction::Inbound { parsed.card } else { None },
        replies: if m.direction == Direction::Inbound { parsed.replies } else { Vec::new() },
        other_metadata: if m.direction == Direction::Inbound { parsed.other } else { None },
        receipt: m.receipt.clone(),
        state: (m.direction == Direction::Outbound).then(|| "accepted".to_string()),
        error: None,
        source: m.source.clone(),
    }
}

fn outbox_view(item: &OutboxItem, index: usize) -> MessageView {
    MessageView {
        id: -(index as i64) - 1,
        local_id: Some(item.local_id.clone()),
        direction: Direction::Outbound,
        from: Some(item.sender.clone()),
        to: item.to.clone(),
        cc: Vec::new(),
        kind: "reply".into(),
        attention: item.attention.clone(),
        tier: Tier::from_attention(&item.attention),
        requires_disposition: false,
        subject: item.subject.clone(),
        body: item.body.clone(),
        sent_at_ms: item.created_at_ms,
        role: "author".into(),
        disposition: None,
        open_obligation: false,
        unread: false,
        card: None,
        replies: Vec::new(),
        other_metadata: None,
        receipt: None,
        state: Some(item.state.clone()),
        error: item.error.clone(),
        source: "authored".into(),
    }
}

/// Thread status and tier, shared by the snapshot and the notification path.
pub struct ThreadState {
    pub status: &'static str,
    pub tier: Tier,
    pub pending: Vec<i64>,
}

pub fn thread_state(
    messages: &[&StoredMessage],
    local: &ThreadLocal,
    has_outbox: bool,
    now: i64,
) -> ThreadState {
    let last_out = messages
        .iter()
        .filter(|m| m.direction == Direction::Outbound)
        .map(|m| m.id)
        .max()
        .unwrap_or(0);
    let resolved_through = local.done_through.max(last_out);
    let pending: Vec<&&StoredMessage> = messages
        .iter()
        .filter(|m| {
            m.direction == Direction::Inbound
                && m.role == "primary"
                && m.id > resolved_through
                && m.tier() != Tier::Fyi
                && !m.is_terminal_disposition()
        })
        .collect();
    let latest_inbound = messages.iter().filter(|m| m.direction == Direction::Inbound).max_by_key(|m| m.id);
    let last_any = messages.iter().map(|m| m.id).max().unwrap_or(0);
    let any_obligating = messages.iter().any(|m| {
        m.direction == Direction::Outbound
            || (m.direction == Direction::Inbound && m.role == "primary" && m.tier() != Tier::Fyi)
    });

    if !pending.is_empty() {
        let tier = pending.iter().map(|m| m.tier()).max().unwrap_or(Tier::Routine);
        let snoozed = local.snooze_until_ms.map_or(false, |until| until > now);
        return ThreadState {
            status: if snoozed { "snoozed" } else { "needs-me" },
            tier,
            pending: pending.iter().map(|m| m.id).collect(),
        };
    }
    let tier = latest_inbound.map(|m| m.tier()).unwrap_or(Tier::Routine);
    if local.pinned {
        // Flagged for follow-up: stays in the inbox until the human clears it.
        return ThreadState { status: "needs-me", tier, pending: Vec::new() };
    }
    let undelivered = messages
        .iter()
        .filter(|m| m.direction == Direction::Outbound && m.id == last_out && last_out == last_any)
        .any(|m| !m.receipt.as_ref().map_or(false, |r| r.consumed));
    let status = if has_outbox || undelivered {
        "waiting"
    } else if !any_obligating {
        "fyi"
    } else {
        "done"
    };
    ThreadState { status, tier, pending: Vec::new() }
}

pub fn thread_counterpart(messages: &[&StoredMessage], outbox: &[&OutboxItem]) -> (String, String) {
    let inbound = messages
        .iter()
        .filter(|m| m.direction == Direction::Inbound)
        .min_by_key(|m| (m.role != "primary", m.id));
    if let Some(m) = inbound {
        return (m.from.clone().unwrap_or_else(|| "(unknown sender)".into()), m.station_address.clone());
    }
    if let Some(m) = messages.iter().min_by_key(|m| m.id) {
        return (m.to.clone(), m.station_address.clone());
    }
    match outbox.first() {
        Some(o) => (o.to.clone(), o.sender.clone()),
        None => ("(unknown)".into(), String::new()),
    }
}

pub struct Extras<'a> {
    pub rev: u64,
    pub station: StationInfo,
    pub presence: &'a BTreeMap<String, Presence>,
    pub health: Vec<HealthView>,
}

pub fn build(store: &Store, extras: Extras<'_>) -> Snapshot {
    let now = now_ms();

    // Resolve every known agent once.
    let agent_project: BTreeMap<String, (Option<String>, String)> =
        store.agents.iter().map(|(addr, a)| (addr.clone(), org::resolve_agent(store, a))).collect();

    let mut by_thread: BTreeMap<i64, Vec<&StoredMessage>> = BTreeMap::new();
    for m in store.messages.values() {
        by_thread.entry(m.thread_id).or_default().push(m);
    }
    let mut outbox_by_thread: BTreeMap<i64, Vec<(usize, &OutboxItem)>> = BTreeMap::new();
    let mut orphan_outbox: Vec<(usize, &OutboxItem)> = Vec::new();
    for (index, item) in store.outbox.iter().enumerate() {
        match item.thread_id {
            Some(t) => outbox_by_thread.entry(t).or_default().push((index, item)),
            None => orphan_outbox.push((index, item)),
        }
    }
    for t in outbox_by_thread.keys() {
        by_thread.entry(*t).or_default();
    }

    let default_local = ThreadLocal::default();
    let mut threads: Vec<ThreadView> = Vec::new();
    for (thread_id, messages) in &by_thread {
        let local = store.threads.get(thread_id).unwrap_or(&default_local);
        let outbox: Vec<(usize, &OutboxItem)> =
            outbox_by_thread.get(thread_id).cloned().unwrap_or_default();
        let outbox_refs: Vec<&OutboxItem> = outbox.iter().map(|(_, o)| *o).collect();
        let state = thread_state(messages, local, !outbox.is_empty(), now);
        let (agent, station_address) = thread_counterpart(messages, &outbox_refs);

        let subject = messages
            .iter()
            .find_map(|m| m.subject.clone().filter(|s| !s.trim().is_empty()))
            .or_else(|| messages.first().map(|m| one_line(&m.body, 70)))
            .or_else(|| outbox_refs.first().map(|o| one_line(&o.body, 70)))
            .unwrap_or_else(|| "(no subject)".into());
        let latest = messages.iter().max_by_key(|m| m.id);
        let snippet = outbox_refs
            .last()
            .map(|o| format!("You: {}", one_line(&o.body, 140)))
            .or_else(|| {
                latest.map(|m| {
                    let prefix = if m.direction == Direction::Outbound { "You: " } else { "" };
                    format!("{prefix}{}", one_line(&m.body, 140))
                })
            })
            .unwrap_or_default();
        let mut views: Vec<MessageView> =
            messages.iter().map(|m| message_view(m, local.read_through)).collect();
        views.sort_by_key(|m| m.id);
        views.extend(outbox.iter().map(|(i, o)| outbox_view(o, *i)));

        let (project_id, project_source) = match &local.project_id {
            Some(id) if org::project_exists(&store.projects, id) => (Some(id.clone()), "thread".to_string()),
            _ => agent_project.get(&agent).cloned().unwrap_or((None, "none".into())),
        };
        let inbound: Vec<&&StoredMessage> =
            messages.iter().filter(|m| m.direction == Direction::Inbound).collect();
        threads.push(ThreadView {
            thread_id: *thread_id,
            subject,
            agent,
            station_address,
            status: state.status.to_string(),
            tier: state.tier,
            unread: inbound.iter().filter(|m| m.id > local.read_through).count() as u32,
            open_obligations: messages.iter().filter(|m| m.open_obligation()).count() as u32,
            last_activity_ms: messages
                .iter()
                .map(|m| m.sent_at_ms)
                .chain(outbox_refs.iter().map(|o| o.created_at_ms))
                .max()
                .unwrap_or(0),
            last_inbound_ms: inbound.iter().map(|m| m.sent_at_ms).max().unwrap_or(0),
            snippet,
            project_id,
            project_source,
            snooze_until_ms: local.snooze_until_ms.filter(|until| *until > now),
            pinned: local.pinned,
            muted: local.muted,
            cc_only: !inbound.is_empty() && inbound.iter().all(|m| m.role != "primary"),
            messages: views,
        });
    }
    // New conversations the human started that Telex has not accepted yet.
    for (index, item) in orphan_outbox {
        threads.push(ThreadView {
            thread_id: -(index as i64) - 1,
            subject: item.subject.clone().unwrap_or_else(|| one_line(&item.body, 70)),
            agent: item.to.clone(),
            station_address: item.sender.clone(),
            status: "waiting".into(),
            tier: Tier::from_attention(&item.attention),
            unread: 0,
            open_obligations: 0,
            last_activity_ms: item.created_at_ms,
            last_inbound_ms: 0,
            snippet: format!("You: {}", one_line(&item.body, 140)),
            project_id: agent_project.get(&item.to).and_then(|p| p.0.clone()),
            project_source: "agent".into(),
            snooze_until_ms: None,
            pinned: false,
            muted: false,
            cc_only: false,
            messages: vec![outbox_view(item, index)],
        });
    }

    let mut counts = Counts::default();
    for t in &threads {
        match t.status.as_str() {
            "needs-me" => {
                counts.needs_me += 1;
                match t.tier {
                    Tier::Blocked => counts.blocked += 1,
                    Tier::NeedsReply => counts.needs_reply += 1,
                    _ => counts.routine += 1,
                }
                if store.settings.policy(t.tier).badge && !t.muted {
                    counts.badge += 1;
                }
                counts.top_tier = Some(counts.top_tier.map_or(t.tier, |x: Tier| x.max(t.tier)));
            }
            "waiting" => counts.waiting += 1,
            "snoozed" => counts.snoozed += 1,
            "fyi" => counts.fyi_unread += t.unread.min(1),
            _ => {}
        }
    }

    let unknown = Presence { state: "unknown".into(), ..Default::default() };
    let mut agents: Vec<AgentView> = store
        .agents
        .values()
        .map(|a| {
            let (project_id, project_source) =
                agent_project.get(&a.address).cloned().unwrap_or((None, "none".into()));
            let mine: Vec<&ThreadView> = threads.iter().filter(|t| t.agent == a.address).collect();
            let needs: Vec<&&ThreadView> = mine.iter().filter(|t| t.status == "needs-me").collect();
            AgentView {
                address: a.address.clone(),
                display_name: a
                    .nickname
                    .clone()
                    .or_else(|| a.card.name.clone())
                    .unwrap_or_else(|| short_address(&a.address)),
                card: a.card.clone(),
                nickname: a.nickname.clone(),
                notes: a.notes.clone(),
                locator_note: a.locator_note.clone(),
                project_id,
                project_source,
                notify: a.notify,
                presence: extras.presence.get(&a.address).cloned().unwrap_or_else(|| unknown.clone()),
                first_seen_ms: a.first_seen_ms,
                last_seen_ms: a.last_seen_ms,
                thread_ids: mine.iter().map(|t| t.thread_id).collect(),
                needs_me: needs.len() as u32,
                top_tier: needs.iter().map(|t| t.tier).max(),
            }
        })
        .collect();
    agents.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));
    counts.unsorted_agents = agents.iter().filter(|a| a.project_id.is_none()).count() as u32;

    let mut notifications: Vec<NotificationRecord> =
        store.notifications.iter().rev().take(150).cloned().collect();
    notifications.sort_by_key(|n| std::cmp::Reverse(n.at_ms));

    Snapshot {
        rev: extras.rev,
        now_ms: now,
        station: extras.station,
        threads,
        agents,
        projects: store.projects.clone(),
        rules: store.rules.clone(),
        settings: store.settings.clone(),
        health: extras.health,
        notifications,
        suggestions: org::suggestions(store),
        counts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: i64, dir: Direction, attention: &str) -> StoredMessage {
        StoredMessage {
            id,
            thread_id: 1,
            parent_id: None,
            direction: dir,
            from: Some(if dir == Direction::Inbound { "agent:a".into() } else { "operator:rob".into() }),
            to: if dir == Direction::Inbound { "operator:rob".into() } else { "agent:a".into() },
            cc: vec![],
            kind: "note".into(),
            attention: attention.into(),
            requires_disposition: false,
            subject: None,
            body: "b".into(),
            metadata: None,
            sent_at_ms: id,
            ingested_at_ms: id,
            role: if dir == Direction::Inbound { "primary".into() } else { "author".into() },
            station_address: "operator:rob".into(),
            delivery: None,
            acked: true,
            disposition: None,
            receipt: None,
            recovery: None,
            source: "receive".into(),
        }
    }

    fn state(messages: &[StoredMessage], local: &ThreadLocal) -> (String, Tier) {
        let refs: Vec<&StoredMessage> = messages.iter().collect();
        let s = thread_state(&refs, local, false, 1_000);
        (s.status.to_string(), s.tier)
    }

    #[test]
    fn ball_in_court_lifecycle() {
        let local = ThreadLocal::default();
        let mut ms = vec![msg(1, Direction::Inbound, "next-checkpoint")];
        assert_eq!(state(&ms, &local), ("needs-me".into(), Tier::NeedsReply));

        // A reply moves the ball; undelivered replies are "waiting".
        let mut reply = msg(2, Direction::Outbound, "next-checkpoint");
        reply.receipt = Some(Receipt::default());
        ms.push(reply);
        assert_eq!(state(&ms, &local).0, "waiting");
        ms[1].receipt.as_mut().unwrap().consumed = true;
        assert_eq!(state(&ms, &local).0, "done");

        // The agent escalates: the thread comes back at the higher tier.
        ms.push(msg(3, Direction::Inbound, "interrupt"));
        assert_eq!(state(&ms, &local), ("needs-me".into(), Tier::Blocked));

        // Snooze hides it until the time passes.
        let snoozed = ThreadLocal { snooze_until_ms: Some(2_000), ..Default::default() };
        assert_eq!(state(&ms, &snoozed).0, "snoozed");
        let expired = ThreadLocal { snooze_until_ms: Some(500), ..Default::default() };
        assert_eq!(state(&ms, &expired).0, "needs-me");

        // Done resolves everything so far.
        let done = ThreadLocal { done_through: 3, ..Default::default() };
        assert_eq!(state(&ms, &done).0, "done");
    }

    #[test]
    fn fyi_and_cc_never_put_the_ball_in_my_court() {
        let local = ThreadLocal::default();
        assert_eq!(state(&[msg(1, Direction::Inbound, "fyi")], &local).0, "fyi");
        let mut cc = msg(1, Direction::Inbound, "interrupt");
        cc.role = "cc".into();
        assert_eq!(state(&[cc], &local).0, "fyi");
    }

    #[test]
    fn terminal_disposition_resolves_a_message() {
        let mut m = msg(1, Direction::Inbound, "interrupt");
        m.requires_disposition = true;
        assert!(m.open_obligation());
        m.disposition = Some("deferred".into());
        assert!(m.open_obligation());
        assert_eq!(state(&[m.clone()], &ThreadLocal::default()).0, "needs-me");
        m.disposition = Some("handled".into());
        assert!(!m.open_obligation());
        assert_eq!(state(&[m], &ThreadLocal::default()).0, "done");
    }
}
