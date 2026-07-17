//! `telex skill`: print the agent usage instructions embedded in the binary, so the only
//! onboarding step is "install telex and run `telex skill`". The content is `SKILL.md`,
//! embedded at compile time, so it always matches this binary's version and features.

use anyhow::Result;

use crate::backend::available_kinds;
use crate::cli::{Ctx, SkillArgs};

const SKILL_MD: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/SKILL.md"));

pub fn raw_skill() -> &'static str {
    SKILL_MD
}

pub async fn run(_ctx: &Ctx, args: SkillArgs) -> Result<i32> {
    if args.raw {
        print!("{}", raw_skill());
        return Ok(0);
    }

    let entra = if cfg!(feature = "entra") {
        "available"
    } else {
        "not in this build"
    };
    println!("telex v{} — agent usage skill", env!("CARGO_PKG_VERSION"));
    println!(
        "(this build: backends [{}]; entra auth {})\n",
        available_kinds().join(", "),
        entra
    );

    if let Some(addr) = &args.address {
        print!("{}", assignment_preamble(addr));
    }

    print!("{}", strip_frontmatter(raw_skill()));
    Ok(0)
}

/// The address-tailored assignment preamble printed before the neutral `SKILL.md` for
/// `telex skill --address <addr>`. It must stay **harness-neutral** (generic
/// attach/wait/ack, `--out-dir` artifacts, no infinite loop) so `telex skill` output —
/// with or without `--address` — carries no harness-specific mechanics; harness push
/// integrations are reached via the `telex <harness> skill` pointer (ADR 0044).
fn assignment_preamble(addr: &str) -> String {
    let mut s = String::new();
    s.push_str("## Your assignment\n\n");
    s.push_str(&format!(
        "You are assigned the telex address `{addr}`. Serve it by registering a station with\n"
    ));
    s.push_str(
        "one-shot `telex attach`; the auto-spawned per-user local exchange owns the lease,\n",
    );
    s.push_str("delivery buffer, and liveness. Then loop one delivery at a time: run a SINGLE\n");
    s.push_str(
        "backgrounded `telex wait --out-dir <dir>` task. Prefer a detached/background task that\n",
    );
    s.push_str(
        "wakes the session on completion, so it does not tie up the terminal like foreground work.\n",
    );
    s.push_str(
        "It writes message.json/status.json/exit.code into <dir>. When the task completes,\n",
    );
    s.push_str("read the artifact exit.code (not the task's own reported exit code); on 0 parse\n");
    s.push_str("message.json, `telex ack`, dedupe by id, then re-arm a fresh `wait`\n");
    s.push_str("before longer processing.\n");
    s.push_str(
        "While focused, arm with `--min-attention interrupt`; at checkpoints drain inbox,\n",
    );
    s.push_str("then re-arm interrupt-only or unfiltered depending on whether you are idle.\n");
    s.push_str("Observer/relay seats that explicitly want live CC traffic to wake them can add\n");
    s.push_str("`--wake-on-cc`; bare waits keep CC pull-only/visibility-only by default.\n");
    s.push_str(&format!(
        "For teardown or upgrade, run `telex station stop --address {addr}` first; it\n"
    ));
    s.push_str("releases the station and waits for tracked live waiters to exit.\n");
    s.push_str("Don't wrap wait in an infinite shell loop (it hides deliveries).\n");
    s.push_str(
        "If your harness has a native telex integration, prefer it: run `telex <harness> skill`\n",
    );
    s.push_str(
        "(e.g. in Copilot CLI, `telex copilot skill`) for the harness-specific workflow.\n\n",
    );
    s.push_str("```sh\n");
    s.push_str(&format!(
        "telex attach --address {addr} --description \"<what you are working on>\"\n"
    ));
    s.push_str(&format!("telex wait --address {addr} --out-dir <dir>\n"));
    s.push_str("```\n\n");
    s
}

/// Drop a leading YAML frontmatter block (`--- ... ---`) so the printed output reads as
/// instructions rather than skill-registration metadata.
fn strip_frontmatter(s: &str) -> &str {
    let trimmed = s.trim_start();
    if let Some(rest) = trimmed.strip_prefix("---") {
        if let Some(idx) = rest.find("\n---") {
            return rest[idx + 4..].trim_start_matches(['\r', '\n']);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_skill_and_strips_frontmatter() {
        // The file is embedded and non-trivial.
        assert!(SKILL_MD.len() > 500);
        let body = strip_frontmatter(SKILL_MD);
        // Frontmatter removed; the document heading remains.
        assert!(!body.trim_start().starts_with("---"));
        assert!(body.contains("telex attach"));
    }

    #[test]
    fn embedded_skill_requires_operator_scannable_subjects() {
        let body = strip_frontmatter(SKILL_MD);
        let normalized = body.split_whitespace().collect::<Vec<_>>().join(" ");
        for required in [
            "concise, non-empty `--subject`",
            "human/operator scan surface",
            "PR #123 ready for review",
            "CI failure needs repair",
            "Issue #45 blocked on scope decision",
            "PR #123 merged; stand down",
            "When the parent already has a useful subject",
            "parent subject is blank, vague, or misleading",
        ] {
            assert!(
                normalized.contains(required),
                "embedded skill must preserve subject guidance {required:?}"
            );
        }
        for line in body
            .lines()
            .filter(|line| line.contains("telex ") && line.contains("send --"))
        {
            assert!(
                line.contains("--subject"),
                "agent-facing send example must include --subject: {line}"
            );
        }
    }

    #[test]
    fn strip_is_noop_without_frontmatter() {
        let s = "# Title\n\nbody";
        assert_eq!(strip_frontmatter(s), s);
    }

    #[test]
    fn assignment_preamble_is_harness_neutral() {
        let p = assignment_preamble("workstream:proj/node:issue-1");
        // Includes the assigned address and the generic loop.
        assert!(p.contains("workstream:proj/node:issue-1"));
        assert!(p.contains("telex attach"));
        assert!(p.contains("telex wait"));
        assert!(p.contains("--out-dir"));
        // Routes to harness-specific skills via the neutral pointer, not inline mechanics.
        assert!(p.contains("telex <harness> skill"));
        // Must carry NO Copilot/harness-specific mechanics (ADR 0044; the `telex skill`
        // acceptance criterion covers `--address` output too).
        for forbidden in [
            "detach: true",
            "detach:true",
            "pwsh -File",
            "pwsh -NoProfile",
            "list_powershell",
            "COPILOT_AGENT_SESSION_ID",
            "COPILOT_LOADER_PID",
            "copilot attach",
            "copilot detach",
            "--copilot-bridge",
            "extensions_reload",
        ] {
            assert!(
                !p.contains(forbidden),
                "address preamble must be harness-neutral; found {forbidden:?}"
            );
        }
    }
}
