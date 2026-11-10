//! Deterministic, bounded fixture for runtime fault and scheduling evidence.

use serde_json::json;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use telex_watcher::protocol_v2::parse_request;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(args.next().expect("fixture root"));
    let mode = args.next().expect("fixture mode").into_string().unwrap();
    assert!(root.is_absolute() && root.is_dir());
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes).unwrap();
    let request = parse_request(&bytes).unwrap();
    let record = json!({
        "request": request,
        "pid": std::process::id(),
        "startTime": telex::session_watch::capture_process_start_time(std::process::id()),
    });
    let writing = root.join(format!("writing-{}", request.attempt.id));
    std::fs::write(&writing, serde_json::to_vec(&record).unwrap()).unwrap();
    std::fs::rename(
        writing,
        root.join(format!("started-{}", request.attempt.id)),
    )
    .unwrap();

    let event = json!({
        "schemaVersion": 2, "outcome": "terminal", "nextState": 1,
        "event": {"kind": "fixture.hardened", "subject": "Crash fixture", "body": "Exact operation"}
    });
    let output = serde_json::to_vec(&event).unwrap();
    match mode.as_str() {
        "terminal" | "event" | "coalescing" => {
            let followup = mode == "coalescing" && request.state == 1;
            let barrier = if followup {
                "release-followup"
            } else {
                "release-all"
            };
            let deadline = Instant::now() + Duration::from_secs(90);
            while !root.join(barrier).is_file() {
                assert!(Instant::now() < deadline, "fixture barrier timed out");
                std::thread::sleep(Duration::from_millis(10));
            }
            if mode == "event" {
                std::io::stdout().write_all(&output).unwrap();
            } else if mode == "coalescing" {
                let result = json!({
                    "schemaVersion": 2,
                    "outcome": if followup { "terminal" } else { "idle" },
                    "nextState": if followup { 2 } else { 1 },
                });
                std::io::stdout()
                    .write_all(&serde_json::to_vec(&result).unwrap())
                    .unwrap();
            } else {
                std::io::stdout()
                    .write_all(br#"{"schemaVersion":2,"outcome":"terminal","nextState":1}"#)
                    .unwrap();
            }
        }
        "malformed" => {
            std::io::stdout()
                .write_all(br#"{"schemaVersion":2,"outcome":"terminal","nextState":1"#)
                .unwrap();
        }
        "nonzero" => {
            std::io::stdout().write_all(&output).unwrap();
            std::io::stderr()
                .write_all(b"fixture nonzero exit")
                .unwrap();
            std::process::exit(23);
        }
        "hang" => {
            // Even complete-looking output is not success before process exit.
            std::io::stdout().write_all(&output).unwrap();
            std::io::stdout().flush().unwrap();
            std::thread::sleep(Duration::from_secs(90));
        }
        "stdout-cap" => {
            std::io::stdout().write_all(&output).unwrap();
            std::io::stdout()
                .write_all(&vec![b' '; telex_watcher::process::MAX_STDOUT_BYTES + 1])
                .unwrap();
        }
        "stderr-cap" => {
            std::io::stdout().write_all(&output).unwrap();
            std::io::stdout().flush().unwrap();
            std::io::stderr()
                .write_all(&vec![b'x'; telex_watcher::process::MAX_STDERR_BYTES + 1])
                .unwrap();
        }
        other => panic!("unknown fixture mode: {other}"),
    }
}
