//! `telex skill`: print the agent usage instructions embedded in the binary, so the only
//! onboarding step is "install telex and run `telex skill`". The content is `SKILL.md`,
//! embedded at compile time, so it always matches this binary's version and features.

use anyhow::Result;

use crate::backend::available_kinds;
use crate::cli::{Ctx, SkillArgs};

const SKILL_MD: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/SKILL.md"));

pub async fn run(_ctx: &Ctx, args: SkillArgs) -> Result<i32> {
    if args.raw {
        print!("{SKILL_MD}");
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
        println!("## Your assignment\n");
        println!(
            "You are assigned the telex address `{addr}`. Serve it by registering a station with"
        );
        println!(
            "one-shot `telex attach`; the auto-spawned per-user local exchange owns the lease,"
        );
        println!("delivery buffer, and liveness. Then loop one delivery at a time: run a SINGLE");
        println!(
            "detached background `telex wait --out-dir <dir>` task named TELEX MESSAGE WAITER."
        );
        println!("It writes message.json/status.json/exit.code into <dir>. When the detached task");
        println!(
            "completes, read the artifact exit.code (not the shell task exit code); on 0 parse"
        );
        println!("message.json, immediately re-arm a fresh detached `wait`, then `telex ack` and");
        println!("act/disposition the delivered message. Use a variable-free command (a literal");
        println!("<dir> path) — detached shell wrappers may strip $variables.");
        println!("Don't wrap wait in an infinite shell loop (it hides deliveries).\n");
        println!("```sh");
        println!("telex attach --address {addr} --description \"<what you are working on>\"");
        println!("telex wait --address {addr} --out-dir <dir>");
        println!("```\n");
    }

    print!("{}", strip_frontmatter(SKILL_MD));
    Ok(0)
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
    fn strip_is_noop_without_frontmatter() {
        let s = "# Title\n\nbody";
        assert_eq!(strip_frontmatter(s), s);
    }
}
