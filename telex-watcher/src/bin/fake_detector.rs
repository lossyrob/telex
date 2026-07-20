//! Deterministic, cross-platform fake detector used by Watcher behavioral tests. It is a trusted
//! test fixture: it reads a JSON config file (which doubles as the hashed "script" so follow-path
//! drift is just an edit of that file), optionally spawns a long-lived descendant to prove
//! process-group termination, sleeps to force timeouts, and emits configurable stdout/stderr and
//! exit codes to exercise idle/event/terminal/degraded/malformed/oversize/non-zero paths.

use fs2::FileExt;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Config {
    #[serde(default)]
    sleep_ms: u64,
    #[serde(default)]
    spawn_child_dir: Option<String>,
    /// Spawns a child that holds an exclusive lock until termination. Windows boundary tests use
    /// this to prove the child ran before timeout and that Job teardown reaped it.
    #[serde(default)]
    spawn_locked_child_dir: Option<String>,
    #[serde(default)]
    child_sleep_ms: u64,
    #[serde(default = "yes")]
    read_stdin: bool,
    #[serde(default)]
    stdout: Option<String>,
    /// When > 0, emit this many bytes of stdout instead of the literal `stdout`.
    #[serde(default)]
    stdout_bytes: usize,
    #[serde(default)]
    stderr: Option<String>,
    #[serde(default)]
    exit_code: i32,
    /// When true, append a byte to the config file (which is the hashed "script") after reading
    /// it, so the runtime observes a deterministic follow-path digest drift for this attempt.
    #[serde(default)]
    mutate_self: bool,
    /// When non-empty, emit an `idle` result whose `nextState.seenEnv` maps each requested name to
    /// whether it was visible in the (cleared + baseline + allowlist) child environment.
    #[serde(default)]
    report_env: Vec<String>,
}

fn yes() -> bool {
    true
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Descendant mode: prove that the whole process group/job is terminated by leaving a
    // "started" marker but never reaching the "finished" marker when killed in time.
    if args.get(1).map(String::as_str) == Some("--child") {
        let dir = args.get(2).cloned().unwrap_or_default();
        let sleep_ms: u64 = args
            .get(3)
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let dir = Path::new(&dir);
        let _ = fs::write(dir.join("child-started"), b"started");
        sleep(Duration::from_millis(sleep_ms));
        let _ = fs::write(dir.join("child-finished"), b"finished");
        return;
    }

    if args.get(1).map(String::as_str) == Some("--locked-child") {
        let dir = args.get(2).cloned().unwrap_or_default();
        let sleep_ms: u64 = args
            .get(3)
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let dir = Path::new(&dir);
        let lock = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(dir.join("child.lock"))
            .expect("open locked-child lock file");
        lock.lock_exclusive().expect("lock locked-child lock file");
        fs::write(dir.join("child-pid"), std::process::id().to_string())
            .expect("write locked-child pid");
        fs::write(dir.join("child-ready"), b"ready").expect("write locked-child readiness");
        sleep(Duration::from_millis(sleep_ms));
        return;
    }

    let config_path = args.get(1).expect("fake_detector requires a config path");
    let config: Config = match fs::read_to_string(config_path) {
        Ok(text) => serde_json::from_str(&text).expect("fake_detector config was not valid JSON"),
        Err(error) => {
            eprintln!("fake_detector could not read config {config_path}: {error}");
            std::process::exit(97);
        }
    };

    if let Some(dir) = &config.spawn_locked_child_dir {
        let exe = std::env::current_exe().expect("resolve fake_detector executable");
        #[allow(clippy::zombie_processes)]
        let _child = Command::new(exe)
            .arg("--locked-child")
            .arg(dir)
            .arg(config.child_sleep_ms.to_string())
            .spawn()
            .expect("spawn locked fake_detector descendant");
    }

    if config.mutate_self {
        use std::io::Write as _;
        if let Ok(mut file) = fs::OpenOptions::new().append(true).open(config_path) {
            let _ = file.write_all(b"\n");
        }
    }

    if let Some(dir) = &config.spawn_child_dir {
        let exe = std::env::current_exe().expect("resolve fake_detector executable");
        // Intentionally do NOT wait: the descendant must outlive this parent so the runtime's
        // process-group/job termination is what kills it. Waiting would block for childSleepMs and
        // defeat the purpose of the fixture.
        #[allow(clippy::zombie_processes)]
        let _child = Command::new(exe)
            .arg("--child")
            .arg(dir)
            .arg(config.child_sleep_ms.to_string())
            .spawn()
            .expect("spawn fake_detector descendant");
    }

    if config.read_stdin {
        let mut buffer = Vec::new();
        let _ = std::io::stdin().read_to_end(&mut buffer);
    }

    if config.sleep_ms > 0 {
        sleep(Duration::from_millis(config.sleep_ms));
    }

    if let Some(text) = &config.stderr {
        let _ = write!(std::io::stderr(), "{text}");
    }

    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    if !config.report_env.is_empty() {
        let mut seen = serde_json::Map::new();
        for name in &config.report_env {
            seen.insert(
                name.clone(),
                serde_json::Value::Bool(std::env::var_os(name).is_some()),
            );
        }
        let result = serde_json::json!({
            "schemaVersion": 1,
            "outcome": "idle",
            "nextState": { "seenEnv": seen },
        });
        let _ = handle.write_all(result.to_string().as_bytes());
        let _ = handle.flush();
        std::process::exit(config.exit_code);
    }
    if config.stdout_bytes > 0 {
        let chunk = vec![b'x'; 8192];
        let mut remaining = config.stdout_bytes;
        while remaining > 0 {
            let take = remaining.min(chunk.len());
            let _ = handle.write_all(&chunk[..take]);
            remaining -= take;
        }
    } else if let Some(text) = &config.stdout {
        let _ = handle.write_all(text.as_bytes());
    }
    let _ = handle.flush();

    std::process::exit(config.exit_code);
}
