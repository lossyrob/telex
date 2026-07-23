use serde_json::Value;
use std::path::Path;
#[cfg(any(unix, windows))]
use std::{
    ffi::OsString,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

#[cfg(any(unix, windows))]
static NEXT_LAUNCHER_TEST: AtomicUsize = AtomicUsize::new(1);

#[test]
// "root skill source" here means the repository-root `SKILL.md` (the canonical embedded
// skill source), which stays at the root; the plugin manifest itself now lives under
// `copilot/plugin/`.
fn plugin_manifest_declares_hooks_and_root_skill_source() {
    let manifest: Value = serde_json::from_str(include_str!("../copilot/plugin/plugin.json"))
        .expect("plugin.json parses");
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
fn hook_manifest_wires_session_end_and_agent_stop_adapters() {
    let hooks: Value = serde_json::from_str(include_str!("../copilot/plugin/hooks.json"))
        .expect("hooks.json parses");
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

    // The idle-drain trigger (issue #65) is a dedicated agentStop hook alongside the turn guard.
    let agent_stop_drain = &hooks["hooks"]["agentStop"][1];
    assert_eq!(agent_stop_drain["type"], "command");
    let powershell = agent_stop_drain["powershell"].as_str().unwrap();
    assert!(powershell.contains("telex --json copilot drain"));
    assert!(!powershell.contains("COPILOT_PLUGIN_ROOT"));
    assert!(!powershell.contains("drain-hook.ps1"));
    let bash = agent_stop_drain["bash"].as_str().unwrap();
    assert!(bash.contains("COPILOT_PLUGIN_ROOT"));
    assert!(bash.contains("drain-hook.sh"));
    assert!(
        bash.contains("then sh "),
        "POSIX launcher must be invoked through sh so executable bits are not required"
    );
    assert!(bash.contains("plugin installation is incomplete"));
    assert!(!bash.contains("telex --json copilot drain"));

    assert!(
        hooks["hooks"].get("notification").is_none(),
        "notification hook is intentionally not installed by default; content enrichment is spike-gated"
    );
}

#[test]
fn drain_hook_launchers_keep_the_plugin_boundary_thin_and_actionable() {
    let hooks: Value =
        serde_json::from_str(include_str!("../copilot/plugin/hooks.json")).expect("hooks json");
    let powershell = hooks["hooks"]["agentStop"][1]["powershell"]
        .as_str()
        .expect("PowerShell hook command");
    let shell = include_str!("../copilot/plugin/drain-hook.sh");
    assert!(powershell.contains("@('off', '0', 'false')"));
    assert!(shell.contains("tr '[:upper:]' '[:lower:]'"));
    assert!(shell.contains("sed 's/^[[:space:]]*//;s/[[:space:]]*$//'"));

    for (name, launcher) in [("PowerShell", powershell), ("POSIX", shell)] {
        assert!(
            launcher.len() < 4096,
            "{name} drain launcher should stay a small plugin boundary"
        );
        assert!(launcher.contains(r#"{}"#));
        assert!(!launcher.contains(r#"{"decision":"allow"}"#));
        assert!(launcher.contains(r#"{"decision":"block","reason":"#));
        assert!(launcher.contains("--json copilot drain"));
        for required in [
            "plugin/binary version skew",
            "binary resolved from PATH could not run",
            "telex copilot drain --help",
            "telex --json version",
            "Get-Command telex",
            "command -v telex",
            "versioned installer",
            "precedes stale shims",
            "matched plugin/binary",
            "restart Copilot",
            "roll back the plugin",
            "TELEX_COPILOT_DRAIN=off",
            "temporary escape hatch",
        ] {
            assert!(
                launcher.contains(required),
                "{name} block decision must include actionable text {required:?}"
            );
        }
    }
}

#[cfg(any(unix, windows))]
#[test]
fn drain_hook_launcher_is_neutral_blocks_and_honors_off_switch() {
    let root = launcher_test_root();
    let fake_bin = root.join("fake-bin");
    std::fs::create_dir_all(&fake_bin).expect("create fake bin");
    write_fake_telex(&fake_bin);
    let capture = root.join("capture.txt");
    let path = prepend_path(&fake_bin);
    let payload = r#"{"sessionId":"launcher-test"}"#;

    let neutral = run_drain_hook(&path, &capture, "0", None, payload);
    assert_neutral_decision(&neutral);
    let captured = std::fs::read_to_string(&capture).expect("read fake telex capture");
    assert!(captured.contains("args=--json copilot drain"));
    assert!(captured.contains("payload={\"sessionId\":\"launcher-test\"}"));

    std::fs::remove_file(&capture).expect("remove allow capture");
    let block = run_drain_hook(&path, &capture, "64", None, payload);
    assert_eq!(
        block.status.code(),
        Some(0),
        "block decision exits successfully"
    );
    assert!(
        block.stderr.is_empty(),
        "launcher stderr must be suppressed: {}",
        String::from_utf8_lossy(&block.stderr)
    );
    let block_stdout = String::from_utf8(block.stdout).expect("block stdout is utf8");
    assert_eq!(
        block_stdout.lines().count(),
        1,
        "launcher must emit exactly one decision: {block_stdout:?}"
    );
    let decision: Value = serde_json::from_str(block_stdout.trim()).expect("block decision json");
    assert_eq!(decision["decision"], "block");
    let reason = decision["reason"].as_str().expect("block reason");
    for required in [
        "plugin/binary version skew",
        "binary resolved from PATH could not run",
        "telex copilot drain --help",
        "telex --json version",
        "Get-Command telex",
        "command -v telex",
        "versioned installer",
        "precedes stale shims",
        "matched plugin/binary",
        "restart Copilot",
        "roll back the plugin",
        "TELEX_COPILOT_DRAIN=off",
        "temporary escape hatch",
    ] {
        assert!(
            reason.contains(required),
            "missing block guidance {required:?}"
        );
    }
    assert!(
        !block_stdout.contains("fake stdout") && !block_stdout.contains("fake stderr"),
        "untrusted adapter output must not leak into the hook decision"
    );

    std::fs::remove_file(&capture).expect("remove block capture");
    let off = run_drain_hook(&path, &capture, "64", Some(" \tFaLsE \t"), payload);
    assert_neutral_decision(&off);
    assert!(
        !capture.exists(),
        "off switch must skip the telex invocation entirely"
    );

    #[cfg(windows)]
    {
        let restricted =
            run_drain_hook_with_execution_policy(&path, &capture, "0", payload, "Restricted");
        assert_neutral_decision(&restricted);
        std::fs::remove_file(&capture).expect("remove restricted-policy capture");

        let missing_bin = root.join("missing-bin");
        std::fs::create_dir_all(&missing_bin).expect("create empty PATH");
        let missing_path = std::env::join_paths([missing_bin]).expect("join empty test PATH");
        let missing = run_drain_hook(&missing_path, &capture, "0", None, payload);
        assert_eq!(
            missing.status.code(),
            Some(0),
            "command-not-found decision exits successfully"
        );
        assert!(missing.stderr.is_empty());
        let decision: Value =
            serde_json::from_slice(&missing.stdout).expect("command-not-found decision json");
        assert_eq!(decision["decision"], "block");
    }

    #[cfg(unix)]
    {
        let missing_launcher = run_drain_hook_missing_plugin_root(payload);
        assert_eq!(missing_launcher.status.code(), Some(0));
        assert!(missing_launcher.stderr.is_empty());
        let decision: Value = serde_json::from_slice(&missing_launcher.stdout)
            .expect("missing-launcher decision json");
        assert_eq!(decision["decision"], "block");
        assert!(decision["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("plugin installation is incomplete")));
    }

    std::fs::remove_dir_all(&root).expect("clean launcher test root");
}

#[test]
fn neutral_drain_output_preserves_other_agent_stop_decisions() {
    let turn_guard_block =
        serde_json::json!({"decision": "block", "reason": "turn guard requires continuation"});
    let drain_neutral = serde_json::json!({});
    let drain_block =
        serde_json::json!({"decision": "block", "reason": "plugin/binary version skew"});
    let competing_block = serde_json::json!({"decision": "block", "reason": "later hook block"});

    let merged = merge_stop_hook_outputs(&[turn_guard_block.clone(), drain_neutral.clone()])
        .expect("turn-guard block survives neutral drain");
    assert_eq!(merged["decision"], "block");
    assert_eq!(merged["reason"], "turn guard requires continuation");

    let merged = merge_stop_hook_outputs(&[drain_neutral, drain_block.clone()])
        .expect("drain block is preserved");
    assert_eq!(merged, drain_block);

    let merged = merge_stop_hook_outputs(&[turn_guard_block, competing_block.clone()])
        .expect("later explicit decision wins");
    assert_eq!(merged, competing_block);
}

fn merge_stop_hook_outputs(outputs: &[Value]) -> Option<Value> {
    outputs
        .iter()
        .filter(|output| output["decision"].is_string())
        .last()
        .cloned()
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
    // harness PR would have to edit — see ADR 0044), enforce the invariant the ADR states:
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
             <harness>/plugin/skills/<name>/SKILL.md bootstrap (see ADR 0044)"
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

    // Regression guard (ADR 0044): the neutral root skill must NOT embed Copilot/harness
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
fn binary_owned_copilot_skill_uses_prepared_cross_platform_fallback() {
    let copilot_skill = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("copilot")
            .join("COPILOT.md"),
    )
    .expect("read binary-owned Copilot skill");
    for required in [
        "copilot fallback prepare",
        "launcher.command",
        "delivery_mode",
        "fully detached",
        "version.build_id",
    ] {
        assert!(
            copilot_skill.contains(required),
            "Copilot skill must document prepared fallback contract {required:?}"
        );
    }

    for removed_manual_wrapper in ["telex-wait-once.ps1", "param("] {
        assert!(
            !copilot_skill.contains(removed_manual_wrapper),
            "Copilot skill must not require the old agent-authored wrapper {removed_manual_wrapper:?}"
        );
    }
}

#[test]
fn bridge_extension_preserves_resumable_registry_and_stops_after_final_unbind() {
    let extension = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("copilot/bridge/extension.mjs"),
    )
    .expect("read bridge extension");
    for required in [
        "TELEX_COPILOT_HOME",
        "bindingsPath",
        "bridgeBindingExists",
        "removeRegistry: !(await bridgeBindingExists())",
        "removeRegistry: true",
        "bridgeProtocol",
        "telexBuildId",
    ] {
        assert!(
            extension.contains(required),
            "bridge extension must preserve lifecycle contract marker {required:?}"
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

#[cfg(any(unix, windows))]
fn launcher_test_root() -> PathBuf {
    let id = NEXT_LAUNCHER_TEST.fetch_add(1, Ordering::SeqCst);
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("copilot-plugin-launcher-tests")
        .join(format!("{}-{id}", std::process::id()));
    if root.exists() {
        std::fs::remove_dir_all(&root).expect("remove stale launcher test root");
    }
    std::fs::create_dir_all(&root).expect("create launcher test root");
    root
}

#[cfg(any(unix, windows))]
fn prepend_path(first: &Path) -> OsString {
    let mut paths = vec![first.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths).expect("join test PATH")
}

#[cfg(windows)]
fn write_fake_telex(dir: &Path) {
    std::fs::write(
        dir.join("telex.cmd"),
        concat!(
            "@echo off\r\n",
            "setlocal\r\n",
            "set \"payload=\"\r\n",
            "set /p \"payload=\"\r\n",
            "> \"%TELEX_FAKE_CAPTURE%\" echo args=%1 %2 %3\r\n",
            ">> \"%TELEX_FAKE_CAPTURE%\" echo payload=%payload%\r\n",
            "echo fake stdout {\"decision\":\"allow\"}\r\n",
            ">&2 echo fake stderr\r\n",
            "exit /b %TELEX_FAKE_EXIT%\r\n",
        ),
    )
    .expect("write fake telex.cmd");
}

#[cfg(unix)]
fn write_fake_telex(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let fake = dir.join("telex");
    std::fs::write(
        &fake,
        concat!(
            "#!/bin/sh\n",
            "payload=\n",
            "IFS= read -r payload || :\n",
            "printf 'args=%s %s %s\\npayload=%s\\n' \"$1\" \"$2\" \"$3\" \"$payload\" > \"$TELEX_FAKE_CAPTURE\"\n",
            "printf '%s\\n' 'fake stdout {\"decision\":\"allow\"}'\n",
            "printf '%s\\n' 'fake stderr' >&2\n",
            "exit \"$TELEX_FAKE_EXIT\"\n",
        ),
    )
    .expect("write fake telex");
    let mut permissions = std::fs::metadata(&fake)
        .expect("fake telex metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&fake, permissions).expect("make fake telex executable");
}

#[cfg(windows)]
fn run_drain_hook(
    path: &OsString,
    capture: &Path,
    fake_exit: &str,
    drain_setting: Option<&str>,
    payload: &str,
) -> Output {
    let powershell = pwsh_path();
    let hooks: Value =
        serde_json::from_str(include_str!("../copilot/plugin/hooks.json")).expect("hooks json");
    let hook_command = hooks["hooks"]["agentStop"][1]["powershell"]
        .as_str()
        .expect("PowerShell hook command");
    let mut command = Command::new(powershell);
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            hook_command,
        ])
        .env("PATH", path)
        .env("TELEX_FAKE_CAPTURE", capture)
        .env("TELEX_FAKE_EXIT", fake_exit);
    match drain_setting {
        Some(value) => {
            command.env("TELEX_COPILOT_DRAIN", value);
        }
        None => {
            command.env_remove("TELEX_COPILOT_DRAIN");
        }
    }
    run_with_stdin(command, payload)
}

#[cfg(windows)]
fn run_drain_hook_with_execution_policy(
    path: &OsString,
    capture: &Path,
    fake_exit: &str,
    payload: &str,
    policy: &str,
) -> Output {
    let hooks: Value =
        serde_json::from_str(include_str!("../copilot/plugin/hooks.json")).expect("hooks json");
    let hook_command = hooks["hooks"]["agentStop"][1]["powershell"]
        .as_str()
        .expect("PowerShell hook command");
    let mut command = Command::new(pwsh_path());
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            policy,
            "-Command",
            hook_command,
        ])
        .env("PATH", path)
        .env("TELEX_FAKE_CAPTURE", capture)
        .env("TELEX_FAKE_EXIT", fake_exit)
        .env_remove("TELEX_COPILOT_DRAIN");
    run_with_stdin(command, payload)
}

#[cfg(windows)]
fn pwsh_path() -> PathBuf {
    let output = Command::new("where.exe")
        .arg("pwsh.exe")
        .output()
        .expect("locate pwsh.exe");
    assert!(
        output.status.success(),
        "pwsh.exe is required for Copilot hook tests"
    );
    String::from_utf8(output.stdout)
        .expect("where pwsh output is utf8")
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| PathBuf::from(line.trim()))
        .expect("where pwsh returned a path")
}

#[cfg(unix)]
fn run_drain_hook(
    path: &OsString,
    capture: &Path,
    fake_exit: &str,
    drain_setting: Option<&str>,
    payload: &str,
) -> Output {
    let hooks: Value =
        serde_json::from_str(include_str!("../copilot/plugin/hooks.json")).expect("hooks json");
    let hook_command = hooks["hooks"]["agentStop"][1]["bash"]
        .as_str()
        .expect("POSIX hook command");
    let mut command = Command::new("sh");
    command
        .args(["-c", hook_command])
        .env(
            "COPILOT_PLUGIN_ROOT",
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("copilot")
                .join("plugin"),
        )
        .env("PATH", path)
        .env("TELEX_FAKE_CAPTURE", capture)
        .env("TELEX_FAKE_EXIT", fake_exit);
    match drain_setting {
        Some(value) => {
            command.env("TELEX_COPILOT_DRAIN", value);
        }
        None => {
            command.env_remove("TELEX_COPILOT_DRAIN");
        }
    }
    run_with_stdin(command, payload)
}

#[cfg(unix)]
fn run_drain_hook_missing_plugin_root(payload: &str) -> Output {
    let hooks: Value =
        serde_json::from_str(include_str!("../copilot/plugin/hooks.json")).expect("hooks json");
    let hook_command = hooks["hooks"]["agentStop"][1]["bash"]
        .as_str()
        .expect("POSIX hook command");
    let mut command = Command::new("sh");
    command
        .args(["-c", hook_command])
        .env_remove("COPILOT_PLUGIN_ROOT");
    run_with_stdin(command, payload)
}

#[cfg(any(unix, windows))]
fn run_with_stdin(mut command: Command, payload: &str) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn drain hook");
    child
        .stdin
        .take()
        .expect("hook stdin")
        .write_all(payload.as_bytes())
        .expect("write hook payload");
    child.wait_with_output().expect("wait for drain hook")
}

#[cfg(any(unix, windows))]
fn assert_neutral_decision(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "neutral decision exits 0");
    assert!(
        output.stderr.is_empty(),
        "launcher stderr must be empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout.clone()).expect("neutral stdout is utf8");
    assert_eq!(
        stdout.trim(),
        r#"{}"#,
        "neutral output must contain only an empty Copilot hook object"
    );
    assert_eq!(
        stdout.lines().count(),
        1,
        "neutral output must be exactly one line"
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
