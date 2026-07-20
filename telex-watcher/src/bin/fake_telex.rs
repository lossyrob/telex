//! Deterministic, cross-platform fake `telex` executable used by Watcher behavioral tests. It
//! persists sender membership as files under `FAKE_TELEX_STATE` so `status` can report the
//! PID-bound predicate the real adapter verifies, and it can be scripted (via state files and
//! env) to return `queued-unoccupied` receipts or a one-shot `NeedsAttach` so the reconcile-once
//! send path is exercised without a real Telex daemon.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn subcommand(args: &[String]) -> Option<&'static str> {
    ["attach", "status", "send", "detach"]
        .into_iter()
        .find(|candidate| args.iter().any(|arg| arg == candidate))
}

fn state_dir() -> PathBuf {
    PathBuf::from(std::env::var("FAKE_TELEX_STATE").expect("FAKE_TELEX_STATE must be set"))
}

fn member_path(dir: &Path, address: &str) -> PathBuf {
    let safe: String = address
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect();
    dir.join(format!("member-{safe}.json"))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = state_dir();
    fs::create_dir_all(&dir).expect("create fake telex state dir");

    match subcommand(&args) {
        Some("attach") => {
            let address = flag_value(&args, "--address").expect("attach requires --address");
            let session = flag_value(&args, "--session").expect("attach requires --session");
            let watch_pid = flag_value(&args, "--watch-pid").expect("attach requires --watch-pid");
            let pid: u64 = watch_pid
                .split(':')
                .next()
                .and_then(|value| value.parse().ok())
                .expect("watch-pid must start with a numeric pid");
            let member = serde_json::json!({
                "address": address,
                "session_id": session,
                "idle": false,
                "watch_pids": [{
                    "pid": pid,
                    "role": "required",
                    "alive": true,
                    "start_time": 123u64,
                }],
            });
            fs::write(member_path(&dir, &address), member.to_string()).unwrap();
            println!("{{\"type\":\"attached\"}}");
        }
        Some("status") => {
            let address = flag_value(&args, "--address").expect("status requires --address");
            let members: Vec<serde_json::Value> = fs::read(member_path(&dir, &address))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .into_iter()
                .collect();
            let response = serde_json::json!({ "daemon_members": members });
            println!("{response}");
        }
        Some("detach") => {
            let address = flag_value(&args, "--address").expect("detach requires --address");
            let _ = fs::remove_file(member_path(&dir, &address));
            println!("{{\"type\":\"detached\"}}");
        }
        Some("send") => {
            let mut stdin = String::new();
            let _ = std::io::stdin().read_to_string(&mut stdin);
            let from = flag_value(&args, "--from").unwrap_or_default();
            let to = flag_value(&args, "--to").unwrap_or_default();

            let needs_attach_once = dir.join("needs-attach-once");
            if needs_attach_once.exists() {
                let _ = fs::remove_file(&needs_attach_once);
                let response = serde_json::json!({
                    "type": "error",
                    "code": "NeedsAttach",
                    "needs_attach_reason": "restart_lost",
                    "message": "membership was lost",
                });
                println!("{response}");
                return;
            }

            if !member_path(&dir, &from).exists() {
                let response = serde_json::json!({
                    "type": "error",
                    "code": "NeedsAttach",
                    "needs_attach_reason": "deliberately_detached",
                    "message": "sender is not attached",
                });
                println!("{response}");
                return;
            }

            let receipt_kind =
                std::env::var("FAKE_TELEX_RECEIPT").unwrap_or_else(|_| "delivered".to_string());
            let response = serde_json::json!({
                "type": "sent",
                "receipt": {
                    "receipt": receipt_kind,
                    "id": 4242,
                    "threadId": 7,
                    "to": to,
                    "from": from,
                },
            });
            println!("{response}");
        }
        None => {
            eprintln!("fake_telex received an unrecognized command: {args:?}");
            std::process::exit(2);
        }
        _ => unreachable!(),
    }
}
