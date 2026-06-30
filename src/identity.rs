//! Resolving a message's `from` (sender) identity for `send` / `reply`.
//!
//! Precedence: explicit `--from` > `$TELEX_ADDRESS` / `--address` > the *uniquely live local
//! station* on the current backend. Explicit/env always win, so the legitimate divergences the
//! issue calls out keep working (a one-shot sender declaring a reply-to it will hold later; a
//! multi-address supervisor; an operator sending as a system address) — `from` is never *forced*
//! to equal the held lease.
//!
//! Guardrails layered on top:
//! - A would-be un-repliable message (`from = None`) that *requires disposition* is **refused** — a
//!   disposition-required message no one can reply to is almost always a mistake.
//! - Inference with more than one live local station is **refused**, listing candidates (no guess).
//! - Otherwise we warn (but proceed) when a message is un-repliable, or when an explicit/env `from`
//!   resolves to an address this host isn't actually serving ("replies will queue unwatched").
//!
//! The policy lives in the pure `plan_from` (unit-tested against every acceptance branch); the async
//! `resolve_from` only gathers the live-holder facts (registry + `ipc::ping`) and delegates.

use anyhow::{anyhow, Result};

use crate::model::Attention;
use crate::registry;

/// Receipt label + exit-code-bearing reason for a refused un-repliable send.
pub const RECEIPT_UNREPLIABLE: &str = "refused-unrepliable";
/// Receipt label for a refused send that couldn't pick among multiple live local stations.
pub const RECEIPT_AMBIGUOUS: &str = "refused-ambiguous-from";

/// Resolve the stable Telex session identity for one-shot daemon verbs.
///
/// Precedence is explicit `--session`, then `TELEX_SESSION_ID`.
/// Copilot harness variables are mapped by the plugin boundary, not by core identity
/// resolution. The helper deliberately fails closed
/// instead of minting a random identity, because `NeedsAttach` recovery must
/// name the same session that originally attached.
pub fn resolve_session_id(explicit: Option<&str>) -> Result<String> {
    optional_session_id(explicit)
        .ok_or_else(|| {
            anyhow!(
                "no session id available; pass --session or set TELEX_SESSION_ID (Copilot users should run the telex Copilot plugin mapper)"
            )
        })
}

pub fn optional_session_id(explicit: Option<&str>) -> Option<String> {
    explicit
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .or_else(|| nonempty_env("TELEX_SESSION_ID"))
}

fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn default_occupant() -> String {
    format!("{}:{}", crate::config::hostname(), std::process::id())
}

/// The outcome of resolving `from` for a `send`/`reply`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromPlan {
    /// Go ahead with this `from` (possibly `None`), emitting `warning` to stderr first if set.
    Proceed {
        from: Option<String>,
        warning: Option<String>,
    },
    /// Refuse the send; `receipt` is the structured label and `message` the human explanation.
    Refuse {
        receipt: &'static str,
        message: String,
    },
}

/// Pure resolution policy. `live_holders` is the set of addresses with a *live* local station on the
/// current backend (already liveness-filtered by the caller); `explicit_from`/`env_address` are the
/// `--from` flag and `$TELEX_ADDRESS`/`--address` value.
pub fn plan_from(
    explicit_from: Option<&str>,
    env_address: Option<&str>,
    live_holders: &[String],
    requires_disposition: bool,
    attention: Attention,
) -> FromPlan {
    // Treat an empty/whitespace-only --from or $TELEX_ADDRESS as unset, so it can't slip past the
    // un-repliable guardrail as a "real" identity that is itself un-repliable (replies to "" go
    // nowhere).
    let explicit_from = explicit_from.filter(|s| !s.trim().is_empty());
    let env_address = env_address.filter(|s| !s.trim().is_empty());

    // Explicit `--from`, then env/`--address`, always win — never forced to the held lease.
    if let Some(addr) = explicit_from.or(env_address) {
        let served = live_holders.iter().any(|h| h == addr);
        let warning = (!served).then(|| {
            format!(
                "sending as {addr} but no live station for {addr} here — replies will queue unwatched"
            )
        });
        return FromPlan::Proceed {
            from: Some(addr.to_string()),
            warning,
        };
    }

    match live_holders {
        // No identity available: guard the dangerous case, warn the merely-imperfect one.
        [] => {
            if requires_disposition {
                FromPlan::Refuse {
                    receipt: RECEIPT_UNREPLIABLE,
                    message:
                        "refusing to send a disposition-required message with no from address: \
                              it would be un-repliable, which is almost always a mistake. Pass \
                              --from <addr>, set $TELEX_ADDRESS, or start a station on an address \
                              first."
                            .to_string(),
                }
            } else if attention != Attention::Fyi {
                FromPlan::Proceed {
                    from: None,
                    warning: Some(
                        "sending with no from address — this message is un-repliable (replies have \
                         nowhere to go). Pass --from <addr> or set $TELEX_ADDRESS to be repliable."
                            .to_string(),
                    ),
                }
            } else {
                // fyi + no disposition: a legitimate fire-and-forget. Stay quiet.
                FromPlan::Proceed {
                    from: None,
                    warning: None,
                }
            }
        }
        // Exactly one live local station: infer it (served locally by construction → no warning).
        [only] => FromPlan::Proceed {
            from: Some(only.clone()),
            warning: None,
        },
        // Ambiguous: refuse and list the candidates rather than guessing an identity.
        many => FromPlan::Refuse {
            receipt: RECEIPT_AMBIGUOUS,
            message: format!(
                "multiple live local stations ({}); cannot infer which to send as. Pass --from \
                 <addr> or set $TELEX_ADDRESS.",
                many.join(", ")
            ),
        },
    }
}

/// Gather live-holder facts and apply [`plan_from`]. When an explicit/env `from` is given we only
/// probe that one address (one `ipc::ping`, off the hot path for configured senders); otherwise we
/// scan the registry for the sole live local station on `backend`.
pub async fn resolve_from(
    explicit_from: Option<&str>,
    env_address: Option<&str>,
    backend: &str,
    requires_disposition: bool,
    attention: Attention,
) -> FromPlan {
    // Mirror plan_from's normalization so we don't ping an empty address.
    let explicit_from = explicit_from.filter(|s| !s.trim().is_empty());
    let env_address = env_address.filter(|s| !s.trim().is_empty());
    let live: Vec<String> = if let Some(addr) = explicit_from.or(env_address) {
        if registry::is_served_locally(addr, backend).await {
            vec![addr.to_string()]
        } else {
            vec![]
        }
    } else {
        registry::live_local_holders(backend).await
    };
    plan_from(
        explicit_from,
        env_address,
        &live,
        requires_disposition,
        attention,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn holders(addrs: &[&str]) -> Vec<String> {
        addrs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn resolve_session_id_does_not_read_copilot_env() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prior_telex = std::env::var_os("TELEX_SESSION_ID");
        let prior_copilot = std::env::var_os("COPILOT_AGENT_SESSION_ID");
        std::env::remove_var("TELEX_SESSION_ID");
        std::env::set_var("COPILOT_AGENT_SESSION_ID", "copilot-session");
        let err = resolve_session_id(None).unwrap_err().to_string();
        restore_env("TELEX_SESSION_ID", prior_telex);
        restore_env("COPILOT_AGENT_SESSION_ID", prior_copilot);
        assert!(err.contains("TELEX_SESSION_ID"));
        assert!(!err.contains("COPILOT_AGENT_SESSION_ID"));
    }

    fn restore_env(key: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    // Criterion: after attach A, send with no --from/env infers from = A.
    #[test]
    fn infers_sole_live_holder() {
        let plan = plan_from(None, None, &holders(&["A"]), false, Attention::Background);
        assert_eq!(
            plan,
            FromPlan::Proceed {
                from: Some("A".to_string()),
                warning: None
            }
        );
    }

    // Criterion: un-repliable AND requires-disposition is refused.
    #[test]
    fn refuses_unrepliable_disposition_required() {
        let plan = plan_from(None, None, &[], true, Attention::Background);
        match plan {
            FromPlan::Refuse { receipt, .. } => assert_eq!(receipt, RECEIPT_UNREPLIABLE),
            other => panic!("expected refuse, got {other:?}"),
        }
    }

    // Criterion: sending as an address not served locally warns but proceeds.
    #[test]
    fn warns_when_explicit_from_not_served_locally() {
        let plan = plan_from(Some("X"), None, &[], false, Attention::Background);
        match plan {
            FromPlan::Proceed {
                from: Some(f),
                warning: Some(w),
            } => {
                assert_eq!(f, "X");
                assert!(w.contains("queue unwatched"), "warning was: {w}");
            }
            other => panic!("expected proceed-with-warning, got {other:?}"),
        }
    }

    #[test]
    fn explicit_from_served_locally_has_no_warning() {
        let plan = plan_from(
            Some("A"),
            None,
            &holders(&["A"]),
            false,
            Attention::Interrupt,
        );
        assert_eq!(
            plan,
            FromPlan::Proceed {
                from: Some("A".to_string()),
                warning: None
            }
        );
    }

    // Criterion: multiple local holders with no --from errors, listing candidates.
    #[test]
    fn refuses_ambiguous_multiple_holders() {
        let plan = plan_from(
            None,
            None,
            &holders(&["A", "B"]),
            false,
            Attention::Background,
        );
        match plan {
            FromPlan::Refuse { receipt, message } => {
                assert_eq!(receipt, RECEIPT_AMBIGUOUS);
                assert!(message.contains('A') && message.contains('B'));
            }
            other => panic!("expected ambiguous refuse, got {other:?}"),
        }
    }

    // Criterion: explicit --from takes precedence over inference.
    #[test]
    fn explicit_from_overrides_inference() {
        let plan = plan_from(
            Some("X"),
            None,
            &holders(&["A"]),
            false,
            Attention::Background,
        );
        match plan {
            FromPlan::Proceed { from: Some(f), .. } => assert_eq!(f, "X"),
            other => panic!("expected from=X, got {other:?}"),
        }
    }

    // Criterion: $TELEX_ADDRESS / --address takes precedence over inference.
    #[test]
    fn env_address_overrides_inference() {
        let plan = plan_from(
            None,
            Some("Y"),
            &holders(&["A"]),
            false,
            Attention::Background,
        );
        match plan {
            FromPlan::Proceed { from: Some(f), .. } => assert_eq!(f, "Y"),
            other => panic!("expected from=Y, got {other:?}"),
        }
    }

    #[test]
    fn fyi_unrepliable_is_silent() {
        let plan = plan_from(None, None, &[], false, Attention::Fyi);
        assert_eq!(
            plan,
            FromPlan::Proceed {
                from: None,
                warning: None
            }
        );
    }

    #[test]
    fn non_fyi_unrepliable_warns_but_proceeds() {
        let plan = plan_from(None, None, &[], false, Attention::NextCheckpoint);
        match plan {
            FromPlan::Proceed {
                from: None,
                warning: Some(w),
            } => assert!(w.contains("un-repliable")),
            other => panic!("expected un-repliable warning, got {other:?}"),
        }
    }

    // Empty/whitespace --from or $TELEX_ADDRESS is normalized to unset, so it is caught by the
    // guardrail instead of slipping through as an un-repliable identity.
    #[test]
    fn empty_explicit_from_is_treated_as_unset_and_refused() {
        let plan = plan_from(Some("  "), None, &[], true, Attention::Background);
        match plan {
            FromPlan::Refuse { receipt, .. } => assert_eq!(receipt, RECEIPT_UNREPLIABLE),
            other => panic!("expected refuse, got {other:?}"),
        }
    }

    #[test]
    fn empty_env_address_falls_through_to_inference() {
        let plan = plan_from(
            None,
            Some(""),
            &holders(&["A"]),
            false,
            Attention::Background,
        );
        assert_eq!(
            plan,
            FromPlan::Proceed {
                from: Some("A".to_string()),
                warning: None
            }
        );
    }
}
