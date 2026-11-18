//! Local notification policy. Pure: Telex attention + local context in, one
//! evidence-bearing decision out. Message content never influences the tier.

use crate::model::{
    now_ms, FocusMode, NotificationRecord, NotifyOverride, Settings, Tier,
};
use chrono::Timelike;

#[derive(Clone, Debug, Default)]
pub struct Context {
    /// Message tier from Telex attention.
    pub tier: Option<Tier>,
    pub is_cc: bool,
    pub thread_muted: bool,
    pub agent_notify: NotifyOverride,
    /// Nearest project override, with the project name for the evidence text.
    pub project_notify: Option<(NotifyOverride, String)>,
    /// Minutes after local midnight; injected so tests are deterministic.
    pub local_minute: Option<u32>,
    pub now_ms: Option<i64>,
}

pub fn local_minute_now() -> u32 {
    let now = chrono::Local::now();
    now.hour() * 60 + now.minute()
}

fn in_window(start: u32, end: u32, minute: u32) -> bool {
    if start == end {
        false
    } else if start < end {
        minute >= start && minute < end
    } else {
        minute >= start || minute < end
    }
}

pub fn focus_active(settings: &Settings, now: i64) -> bool {
    settings.focus.mode != FocusMode::Off && settings.focus.until_ms.map_or(true, |until| now < until)
}

#[allow(clippy::too_many_arguments)]
pub fn decide(
    settings: &Settings,
    ctx: &Context,
    kind: &str,
    thread_id: Option<i64>,
    message_id: Option<i64>,
    title: String,
    body: String,
) -> NotificationRecord {
    let tier = ctx.tier.unwrap_or(Tier::Routine);
    let now = ctx.now_ms.unwrap_or_else(now_ms);
    let mut effective = tier;
    let mut quiet_reason: Option<String> = None;
    let mut notes: Vec<String> = Vec::new();

    // Most specific override wins: thread, then agent, then nearest project.
    let (over, source) = if ctx.thread_muted {
        (NotifyOverride::Mute, "thread".to_string())
    } else if ctx.agent_notify != NotifyOverride::Inherit {
        (ctx.agent_notify, "agent".to_string())
    } else if let Some((o, name)) = &ctx.project_notify {
        (*o, format!("project “{name}”"))
    } else {
        (NotifyOverride::Inherit, String::new())
    };
    match over {
        NotifyOverride::Inherit => {}
        NotifyOverride::Mute => quiet_reason = Some(format!("muted by {source}")),
        NotifyOverride::BlockedOnly => {
            if tier < Tier::Blocked {
                quiet_reason = Some(format!("{source} only interrupts for Blocked"));
            }
        }
        NotifyOverride::Elevate => {
            if tier == Tier::Routine {
                effective = Tier::NeedsReply;
                notes.push(format!("elevated by {source}"));
            }
        }
    }

    let policy = settings.policy(effective).clone();
    let (mut banner, mut sound, mut demand, mut badge) =
        (policy.banner, policy.sound, policy.demand, policy.badge);

    if ctx.is_cc {
        // CC creates no human workflow obligation.
        quiet_reason = Some("cc delivery: no obligation".into());
        badge = false;
    }
    if quiet_reason.is_none() && focus_active(settings, now) {
        match settings.focus.mode {
            FocusMode::BlockedOnly if effective < Tier::Blocked => {
                quiet_reason = Some("focus: only Blocked interrupts".into())
            }
            FocusMode::Silent => quiet_reason = Some("focus: silent".into()),
            _ => {}
        }
    }
    if quiet_reason.is_none() && settings.quiet_hours.enabled {
        let minute = ctx.local_minute.unwrap_or_else(local_minute_now);
        if in_window(settings.quiet_hours.start_min, settings.quiet_hours.end_min, minute)
            && !(settings.quiet_hours.allow_blocked && effective == Tier::Blocked)
        {
            quiet_reason = Some("quiet hours".into());
        }
    }
    if quiet_reason.is_some() {
        banner = false;
        sound = false;
        demand = false;
    }

    let (outcome, reason) = match (&quiet_reason, banner) {
        (Some(reason), _) => ("quiet", reason.clone()),
        (None, true) => ("shown", format!("{} policy", effective.label())),
        (None, false) => ("quiet", format!("{} policy: no banner", effective.label())),
    };
    let reason = if notes.is_empty() { reason } else { format!("{reason}; {}", notes.join("; ")) };

    NotificationRecord {
        id: format!("n-{}", uuid::Uuid::new_v4().simple()),
        at_ms: now,
        kind: kind.to_string(),
        thread_id,
        message_id,
        tier,
        effective_tier: effective,
        title,
        body,
        banner,
        sound: banner && sound,
        demand: banner && demand,
        badge,
        outcome: outcome.to_string(),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Focus, QuietHours};

    fn run(settings: &Settings, ctx: Context) -> NotificationRecord {
        decide(settings, &ctx, "new", Some(1), Some(1), "t".into(), "b".into())
    }
    fn ctx(tier: Tier) -> Context {
        Context { tier: Some(tier), local_minute: Some(12 * 60), now_ms: Some(1_000), ..Default::default() }
    }

    #[test]
    fn default_matrix() {
        let s = Settings::default();
        let blocked = run(&s, ctx(Tier::Blocked));
        assert!(blocked.banner && blocked.sound && blocked.demand && blocked.badge);
        let needs = run(&s, ctx(Tier::NeedsReply));
        assert!(needs.banner && !needs.sound && !needs.demand && needs.badge);
        let routine = run(&s, ctx(Tier::Routine));
        assert!(!routine.banner && routine.badge);
        assert_eq!(routine.outcome, "quiet");
        let fyi = run(&s, ctx(Tier::Fyi));
        assert!(!fyi.banner && !fyi.badge);
    }

    #[test]
    fn cc_never_interrupts_or_badges() {
        let s = Settings::default();
        let r = run(&s, Context { is_cc: true, ..ctx(Tier::Blocked) });
        assert!(!r.banner && !r.badge);
        assert!(r.reason.contains("cc"));
    }

    #[test]
    fn focus_modes() {
        let mut s = Settings::default();
        s.focus = Focus { mode: FocusMode::BlockedOnly, until_ms: Some(5_000) };
        assert!(!run(&s, ctx(Tier::NeedsReply)).banner);
        assert!(run(&s, ctx(Tier::Blocked)).banner);
        // Focus expires on its own.
        let later = Context { now_ms: Some(6_000), ..ctx(Tier::NeedsReply) };
        assert!(run(&s, later).banner);
        s.focus = Focus { mode: FocusMode::Silent, until_ms: None };
        let r = run(&s, ctx(Tier::Blocked));
        assert!(!r.banner && r.badge);
    }

    #[test]
    fn quiet_hours_wrap_midnight_and_let_blocked_through() {
        let mut s = Settings::default();
        s.quiet_hours = QuietHours { enabled: true, start_min: 22 * 60, end_min: 7 * 60, allow_blocked: true };
        let night = |tier| Context { local_minute: Some(23 * 60), ..ctx(tier) };
        assert!(!run(&s, night(Tier::NeedsReply)).banner);
        assert!(run(&s, night(Tier::Blocked)).banner);
        let early = Context { local_minute: Some(6 * 60), ..ctx(Tier::NeedsReply) };
        assert!(!run(&s, early).banner);
        assert!(run(&s, ctx(Tier::NeedsReply)).banner);
    }

    #[test]
    fn overrides_most_specific_wins() {
        let s = Settings::default();
        let muted_project = Context {
            project_notify: Some((NotifyOverride::Mute, "docs".into())),
            ..ctx(Tier::Blocked)
        };
        let r = run(&s, muted_project.clone());
        assert!(!r.banner && r.badge && r.reason.contains("docs"));

        // Agent override outranks the project's.
        let elevated = Context { agent_notify: NotifyOverride::Elevate, ..muted_project };
        let mut c = elevated.clone();
        c.tier = Some(Tier::Routine);
        let r = run(&s, c);
        assert!(r.banner);
        assert_eq!(r.effective_tier, Tier::NeedsReply);
        assert_eq!(r.tier, Tier::Routine);

        let thread = Context { thread_muted: true, ..elevated };
        assert!(!run(&s, thread).banner);

        let blocked_only =
            Context { agent_notify: NotifyOverride::BlockedOnly, ..ctx(Tier::NeedsReply) };
        assert!(!run(&s, blocked_only).banner);
    }
}
