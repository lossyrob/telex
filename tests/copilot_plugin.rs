use serde_json::Value;
use std::path::Path;

#[test]
// "root skill source" here means the repository-root `SKILL.md` (the canonical embedded
// skill source), which stays at the root; the plugin manifest itself now lives under
// `copilot/plugin/`.
fn plugin_manifest_declares_hooks_and_root_skill_source() {
    let manifest: Value =
        serde_json::from_str(include_str!("../copilot/plugin/plugin.json")).expect("plugin.json parses");
    assert_eq!(manifest["name"], "telex");
    assert_eq!(manifest["hooks"], "hooks.json");
    assert_eq!(
        manifest["skills"], "skills/",
        "plugin skill discovery uses Copilot's supported plugin skills directory"
    );
    assert!(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("SKILL.md")
            .exists(),
        "root SKILL.md is the canonical skill source"
    );
}

#[test]
fn marketplace_manifest_advertises_telex_plugin() {
    let marketplace: Value =
        serde_json::from_str(include_str!("../.github/plugin/marketplace.json"))
            .expect("marketplace.json parses");
    assert_eq!(marketplace["name"], "telex");
    let plugins = marketplace["plugins"].as_array().expect("plugins array");
    assert_eq!(plugins.len(), 1);
    let plugin = &plugins[0];
    assert_eq!(plugin["name"], "telex");
    assert_eq!(plugin["source"], "copilot/plugin");
    assert_eq!(plugin["repository"], "https://github.com/lossyrob/telex");
    // Couple the marketplace source string to the on-disk plugin root so a future
    // rename that touches only one side fails at repo-test time, not at a user's
    // `copilot plugin install`.
    let source = plugin["source"].as_str().expect("source is a string");
    assert!(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(source)
            .join("plugin.json")
            .exists(),
        "marketplace source {source:?} must contain the plugin manifest"
    );
}

#[test]
fn hook_manifest_wires_session_end_and_agent_stop_to_hidden_rust_adapter() {
    let hooks: Value =
        serde_json::from_str(include_str!("../copilot/plugin/hooks.json")).expect("hooks.json parses");
    assert_eq!(hooks["version"], 1);
    let session_end = &hooks["hooks"]["sessionEnd"][0];
    assert_eq!(session_end["type"], "command");
    assert!(session_end["powershell"]
        .as_str()
        .unwrap()
        .contains("telex --json copilot session-end"));
    assert!(session_end["bash"]
        .as_str()
        .unwrap()
        .contains("telex --json copilot session-end"));

    let agent_stop = &hooks["hooks"]["agentStop"][0];
    assert_eq!(agent_stop["type"], "command");
    assert!(agent_stop["powershell"]
        .as_str()
        .unwrap()
        .contains("telex --json copilot turn-guard"));
    assert!(agent_stop["bash"]
        .as_str()
        .unwrap()
        .contains("telex --json copilot turn-guard"));

    assert!(
        hooks["hooks"].get("notification").is_none(),
        "notification hook is intentionally not installed by default; content enrichment is spike-gated"
    );
}

#[test]
fn plugin_skill_is_thin_bootstrap_that_defers_to_the_binary() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut skill_files = Vec::new();
    collect_skill_files(root, &mut skill_files);
    let root_skill = root.join("SKILL.md");
    let plugin_skill = root
        .join("copilot")
        .join("plugin")
        .join("skills")
        .join("telex")
        .join("SKILL.md");
    // Today the only skill files are the neutral root SKILL.md and the Copilot plugin
    // bootstrap. Rather than pin the exact 2-element snapshot (which the first sibling
    // harness PR would have to edit — see ADR 0043), enforce the invariant the ADR states:
    // the neutral root skill exists, the Copilot bootstrap exists, and every skill file is
    // either the root skill or a `<harness>/plugin/skills/<name>/SKILL.md` bootstrap. This
    // still catches a stray SKILL.md copied somewhere unexpected while allowing siblings.
    // NOTE: this is a stray-file / layout-shape guard, not a full sibling-harness contract
    // check — the first sibling-harness PR should add its own manifest/skill assertions.
    assert!(
        skill_files.contains(&root_skill),
        "the canonical root SKILL.md must exist"
    );
    assert!(
        skill_files.contains(&plugin_skill),
        "the Copilot plugin bootstrap must exist at copilot/plugin/skills/telex/SKILL.md"
    );
    for f in &skill_files {
        let rel = f.strip_prefix(root).unwrap_or(f);
        let comps: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        // `<harness>/plugin/skills/<name>/SKILL.md`
        let is_harness_bootstrap = comps.len() == 5
            && comps[1] == "plugin"
            && comps[2] == "skills"
            && comps[4] == "SKILL.md";
        assert!(
            f == &root_skill || is_harness_bootstrap,
            "unexpected SKILL.md at {rel:?}: skill files must be the neutral root skill or a \
             <harness>/plugin/skills/<name>/SKILL.md bootstrap (see ADR 0043)"
        );
    }

    let plugin_text = std::fs::read_to_string(&plugin_skill).expect("read plugin skill");

    // The bootstrap is deliberately small and is NOT a copy of the canonical skill.
    // A fixed byte ceiling expresses the "thin bootstrap" intent directly, rather than a
    // ratio against the (also-edited) root skill whose denominator can drift.
    const BOOTSTRAP_MAX_BYTES: usize = 4096;
    assert!(
        plugin_text.len() < BOOTSTRAP_MAX_BYTES,
        "plugin skill should be a thin bootstrap (< {BOOTSTRAP_MAX_BYTES} bytes), got {} bytes",
        plugin_text.len()
    );

    // It defers to the installed binary as the source of truth.
    assert!(
        plugin_text.contains("telex copilot skill"),
        "bootstrap must point Copilot sessions at `telex copilot skill`"
    );
    assert!(
        plugin_text.contains("telex copilot --help"),
        "bootstrap must name command help as the syntax source of truth"
    );
    assert!(
        plugin_text.contains("telex --version"),
        "bootstrap must tell the agent to check the installed version"
    );

    // It must NOT embed the detailed recipes / flag matrices that belong to the binary.
    for forbidden in [
        "## Command reference",
        "Detached waiter pattern",
        "## Attention levels",
        "## Disposition states",
        "pwsh -File",
        "list_powershell",
    ] {
        assert!(
            !plugin_text.contains(forbidden),
            "bootstrap should not embed detailed section {forbidden:?}; that lives in the binary"
        );
    }
}

#[test]
fn root_skill_points_copilot_sessions_at_the_binary_command() {
    let root_skill =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("SKILL.md"))
            .expect("read root skill");
    // The harness-neutral root skill names `telex copilot skill` as the example of the
    // generic `telex <harness> skill` pointer. When a second harness ships, extend this
    // to assert its `telex <harness> skill` pointer too — keep the root skill neutral:
    // it should route to the binary, not embed per-harness mechanics.
    assert!(
        root_skill.contains("telex copilot skill"),
        "root skill should route the Copilot session to `telex copilot skill`"
    );

    // Regression guard (ADR 0043): the neutral root skill must NOT embed Copilot/harness
    // mechanics. A positive pointer assertion alone would still pass if Copilot recipes
    // drifted back in, which is exactly the high-risk regression. The address-tailored
    // preamble printed for `telex skill --address` is guarded separately by
    // `commands::skill::tests::assignment_preamble_is_harness_neutral`.
    for forbidden in [
        "$COPILOT_AGENT_SESSION_ID",
        "COPILOT_LOADER_PID",
        "telex copilot detach",
        "copilot attach --copilot-bridge",
        "extensions_reload",
        "pwsh -File",
        "list_powershell",
        "detach: true",
    ] {
        assert!(
            !root_skill.contains(forbidden),
            "root SKILL.md must stay harness-neutral; found Copilot mechanic {forbidden:?}"
        );
    }
}

#[test]
fn plugin_version_is_consistent_across_manifest_marketplace_and_bootstrap() {
    let manifest: Value =
        serde_json::from_str(include_str!("../copilot/plugin/plugin.json")).expect("plugin.json");
    let marketplace: Value =
        serde_json::from_str(include_str!("../.github/plugin/marketplace.json"))
            .expect("marketplace.json");
    let manifest_version = manifest["version"].as_str().expect("manifest version");
    let marketplace_version = marketplace["plugins"][0]["version"]
        .as_str()
        .expect("marketplace plugin version");
    assert_eq!(
        manifest_version, marketplace_version,
        "plugin.json and marketplace.json plugin versions must match"
    );
    // The bootstrap's compatibility-check example pins the same version.
    let bootstrap = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("copilot")
            .join("plugin")
            .join("skills")
            .join("telex")
            .join("SKILL.md"),
    )
    .expect("read bootstrap");
    assert!(
        bootstrap.contains(&format!("--plugin-version {manifest_version}")),
        "bootstrap `--plugin-version` example must match plugin.json version {manifest_version}"
    );
}

fn collect_skill_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let entries = std::fs::read_dir(dir).expect("read dir");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.components().any(|c| {
            let c = c.as_os_str();
            c == "target"
                || c == ".git"
                || c == ".paw"
                || c == ".streamliner"
                || c == "node_modules"
                || c == ".venv"
                || c == "dist"
        }) {
            continue;
        }
        if path.is_dir() {
            collect_skill_files(&path, out);
        } else if path.file_name().and_then(|n| n.to_str()) == Some("SKILL.md") {
            out.push(path);
        }
    }
    out.sort();
}
