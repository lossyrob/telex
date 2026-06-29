use serde_json::Value;
use std::path::Path;

#[test]
fn plugin_manifest_declares_hooks_and_root_skill_source() {
    let manifest: Value =
        serde_json::from_str(include_str!("../plugin.json")).expect("plugin.json parses");
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
    let marketplace: Value = serde_json::from_str(include_str!("../.github/plugin/marketplace.json"))
        .expect("marketplace.json parses");
    assert_eq!(marketplace["name"], "telex");
    let plugins = marketplace["plugins"].as_array().expect("plugins array");
    assert_eq!(plugins.len(), 1);
    let plugin = &plugins[0];
    assert_eq!(plugin["name"], "telex");
    assert_eq!(plugin["source"], ".");
    assert_eq!(plugin["repository"], "https://github.com/lossyrob/telex");
}

#[test]
fn hook_manifest_wires_session_end_and_agent_stop_to_hidden_rust_adapter() {
    let hooks: Value =
        serde_json::from_str(include_str!("../hooks.json")).expect("hooks.json parses");
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
fn plugin_skill_mirror_matches_root_skill_exactly() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut skill_files = Vec::new();
    collect_skill_files(root, &mut skill_files);
    let root_skill = root.join("SKILL.md");
    let plugin_skill = root.join("skills").join("telex").join("SKILL.md");
    assert_eq!(
        skill_files,
        vec![root_skill.clone(), plugin_skill.clone()],
        "only the canonical root skill and plugin skill mirror should exist"
    );
    assert_eq!(
        std::fs::read(&root_skill).expect("read root skill"),
        std::fs::read(&plugin_skill).expect("read plugin skill"),
        "plugin skill mirror must stay byte-identical to root SKILL.md"
    );
}

fn collect_skill_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let entries = std::fs::read_dir(dir).expect("read dir");
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .components()
            .any(|c| c.as_os_str() == "target" || c.as_os_str() == ".git")
        {
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
