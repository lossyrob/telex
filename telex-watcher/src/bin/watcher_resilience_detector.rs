//! Barrier-controlled event detector for isolated runtime recovery tests.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use telex_watcher::protocol_v2::parse_request;

fn main() {
    let directory = PathBuf::from(std::env::args_os().nth(1).expect("fixture directory"));
    assert!(directory.is_absolute());
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes).unwrap();
    let request = parse_request(&bytes).unwrap();
    let cursor = request.state.as_u64().expect("numeric fixture cursor");
    std::fs::write(
        directory.join(format!("started-{}", request.attempt.id)),
        &bytes,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(90);
    while !directory.join(format!("permit-{cursor}")).exists() {
        assert!(Instant::now() < deadline, "detector barrier timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
    let result = serde_json::json!({
        "schemaVersion": 2,
        "outcome": "event",
        "nextState": cursor + 1,
        "event": {
            "kind": "fixture.recovered",
            "subject": "Recovery fixture",
            "body": format!("cursor {cursor}")
        }
    });
    std::io::stdout()
        .write_all(serde_json::to_string(&result).unwrap().as_bytes())
        .unwrap();
}
