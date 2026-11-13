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
    #[cfg(unix)]
    if let Some(code) = unix_modes::early(mode, &arguments) {
        std::process::exit(code);
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
        #[cfg(unix)]
        other if unix_modes::root(other, &arguments) => {}
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

/// Conforming and deliberately nonconforming process shapes for the macOS
/// assigned-process-group conformance suite. Every lingering process here has
/// a finite lifetime or stays inside the group that containment owns.
#[cfg(unix)]
mod unix_modes {
    use super::{forever, wait_for};
    use std::ffi::CString;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    fn c(value: &str) -> CString {
        CString::new(value).unwrap()
    }

    fn write_pid(root: &Path, name: &str) {
        let path = root.join(format!("{name}.pid"));
        let partial = root.join(format!("{name}.pid.{}", std::process::id()));
        std::fs::write(&partial, std::process::id().to_string()).unwrap();
        std::fs::rename(partial, path).unwrap();
    }

    /// Start `self <arguments>` by one of the three public creation routes.
    fn start(route: usize, arguments: &[&str]) {
        let exe = c(std::env::current_exe().unwrap().to_str().unwrap());
        let owned: Vec<CString> = std::iter::once(exe.clone())
            .chain(arguments.iter().map(|argument| c(argument)))
            .collect();
        let mut argv: Vec<*const libc::c_char> = owned.iter().map(|a| a.as_ptr()).collect();
        argv.push(std::ptr::null());
        extern "C" {
            static environ: *const *const libc::c_char;
            // Public (deprecated on macOS) vfork; not exported by every libc
            // crate target.
            fn vfork() -> libc::pid_t;
        }
        unsafe {
            match route % 3 {
                0 => {
                    let pid = libc::fork();
                    assert!(pid >= 0);
                    if pid == 0 {
                        libc::execv(exe.as_ptr(), argv.as_ptr());
                        libc::_exit(127);
                    }
                }
                1 => {
                    let mut pid = 0;
                    assert_eq!(
                        libc::posix_spawn(
                            &mut pid,
                            exe.as_ptr(),
                            std::ptr::null(),
                            std::ptr::null(),
                            argv.as_ptr() as *const *mut libc::c_char,
                            environ as *const *mut libc::c_char,
                        ),
                        0
                    );
                }
                _ => {
                    let pid = vfork();
                    assert!(pid >= 0);
                    if pid == 0 {
                        libc::execv(exe.as_ptr(), argv.as_ptr());
                        libc::_exit(127);
                    }
                }
            }
        }
    }

    /// Modes that must not read stdin first.
    pub(super) fn early(mode: &str, arguments: &[String]) -> Option<i32> {
        match mode {
            // gen <root> <depth>: one link of a multi-generation chain.
            "gen" => {
                let root = PathBuf::from(&arguments[2]);
                let depth: usize = arguments[3].parse().unwrap();
                write_pid(&root, &format!("gen-{depth}"));
                if depth > 0 {
                    start(depth, &["gen", &arguments[2], &(depth - 1).to_string()]);
                }
                forever();
            }
            // reexec <root> <name>: a member that keeps replacing its image.
            "reexec" => {
                let root = PathBuf::from(&arguments[2]);
                if !root.join(format!("{}.pid", arguments[3])).exists() {
                    write_pid(&root, &arguments[3]);
                }
                std::thread::sleep(Duration::from_millis(2));
                let exe = c(std::env::current_exe().unwrap().to_str().unwrap());
                let owned: Vec<CString> = arguments.iter().map(|a| c(a)).collect();
                let mut argv: Vec<*const libc::c_char> = owned.iter().map(|a| a.as_ptr()).collect();
                argv.push(std::ptr::null());
                unsafe { libc::execv(exe.as_ptr(), argv.as_ptr()) };
                Some(127)
            }
            // escaped <root> <seconds>: leaves the assigned group (and so the
            // contract), keeps the inherited pipes, and ends on its own.
            "escaped" => {
                assert!(unsafe { libc::setsid() } >= 0);
                let root = PathBuf::from(&arguments[2]);
                write_pid(&root, "escaped");
                std::thread::sleep(Duration::from_secs(arguments[3].parse().unwrap()));
                Some(0)
            }
            _ => None,
        }
    }

    /// Detector-root modes, entered after stdin has been consumed.
    pub(super) fn root(mode: &str, arguments: &[String]) -> bool {
        let Some(root) = arguments.get(2).map(PathBuf::from) else {
            return false;
        };
        match mode {
            "gens" | "gens-return" => {
                write_pid(&root, "root");
                start(0, &["gen", &arguments[2], "3"]);
                wait_for(&root.join("gen-0.pid"));
                if mode == "gens" {
                    forever();
                }
                println!("{{\"generations\":\"complete\"}}");
            }
            "churn" => {
                write_pid(&root, "root");
                for index in 0..4 {
                    start(
                        index,
                        &["reexec", &arguments[2], &format!("reexec-{index}")],
                    );
                }
                start(0, &["gen", &arguments[2], "3"]);
                for index in 0..4 {
                    wait_for(&root.join(format!("reexec-{index}.pid")));
                }
                wait_for(&root.join("gen-0.pid"));
                forever();
            }
            "detach" => {
                write_pid(&root, "root");
                start(0, &["escaped", &arguments[2], &arguments[3]]);
                wait_for(&root.join("escaped.pid"));
                println!("{{\"detached\":\"complete\"}}");
            }
            "linger" => {
                write_pid(&root, "root");
                std::thread::sleep(Duration::from_secs(arguments[3].parse().unwrap()));
            }
            _ => return false,
        }
        true
    }
}

#[cfg(unix)]
fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(Instant::now() < deadline, "descendant startup timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}
