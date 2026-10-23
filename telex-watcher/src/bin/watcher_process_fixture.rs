//! Isolated process-tree fixture. Never invokes Telex or accesses a user registry.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn main() {
    for name in ["TELEX_HOME", "TELEX_DB", "TELEX_INSTALL_ROOT"] {
        assert!(
            Path::new(&std::env::var_os(name).expect("isolated fixture environment")).is_absolute()
        );
    }
    let arguments: Vec<_> = std::env::args().collect();
    let mode = arguments.get(1).expect("fixture mode");
    if mode == "stdin-block" {
        forever();
    }
    if mode == "flood-out" {
        loop {
            std::io::stdout().write_all(&[b'x'; 8192]).unwrap();
        }
    }
    if mode == "flood-err" {
        loop {
            std::io::stderr().write_all(&[b'x'; 8192]).unwrap();
        }
    }
    if mode == "leaf" || mode == "child" {
        let root = PathBuf::from(&arguments[2]);
        #[cfg(target_os = "linux")]
        if mode == "child" {
            assert!(unsafe { libc::setsid() } >= 0);
        }
        std::fs::write(
            root.join(format!("{mode}.pid")),
            std::process::id().to_string(),
        )
        .unwrap();
        if mode == "child" {
            spawn("leaf", &root);
        }
        forever();
    }
    let mut request = Vec::new();
    std::io::stdin().read_to_end(&mut request).unwrap();
    match mode.as_str() {
        "emit" => {
            println!(
                "{}",
                serde_json::json!({
                    "cwd": std::env::current_dir().unwrap(),
                    "arguments": &arguments[2..],
                    "input": String::from_utf8(request).unwrap(),
                    "allowed": std::env::var("WATCHER_TEST_ALLOWED").ok(),
                    "ambient": std::env::var("WATCHER_TEST_AMBIENT").ok(),
                    "home": std::env::var("TELEX_HOME").unwrap(),
                    "db": std::env::var("TELEX_DB").unwrap(),
                    "installRoot": std::env::var("TELEX_INSTALL_ROOT").unwrap(),
                })
            );
            if let Ok(secret) = std::env::var("WATCHER_TEST_TOKEN") {
                eprintln!("token={secret}");
            }
        }
        "fail" => {
            if let Ok(secret) = std::env::var("WATCHER_TEST_TOKEN") {
                eprintln!("token={secret}");
            }
            std::process::exit(7);
        }
        "marker" => std::fs::write(&arguments[2], b"executed").unwrap(),
        "tree" | "tree-return" => {
            let root = PathBuf::from(&arguments[2]);
            std::fs::write(root.join("root.pid"), std::process::id().to_string()).unwrap();
            spawn("child", &root);
            let deadline = Instant::now() + Duration::from_secs(10);
            while !root.join("leaf.pid").exists() {
                assert!(Instant::now() < deadline, "descendant startup timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
            if mode == "tree" {
                forever();
            }
            println!("{{\"tree\":\"complete\"}}");
        }
        _ => panic!("unknown fixture mode"),
    }
}

#[allow(
    clippy::zombie_processes,
    reason = "This fixture deliberately leaves descendants for the containment owner to reap."
)]
fn spawn(mode: &str, root: &Path) {
    Command::new(std::env::current_exe().unwrap())
        .arg(mode)
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
}

fn forever() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
