//! Native macOS assigned-process-group conformance (ADR 0054 / ADR 0057).
//!
//! These cases run the shipped guard on the host they execute on. They are
//! evidence for that host only; passing here is not a runtime allowlist.
//! Every process a test signals is the test's own unreaped direct child.

use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::sync::{Arc, Mutex};

fn guard() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_watcher_process_guard"))
}

fn pid_of(root: &Path, name: &str) -> u32 {
    let path = root.join(format!("{name}.pid"));
    wait_file(&path);
    std::fs::read_to_string(path).unwrap().parse().unwrap()
}

fn assert_ended(root: &Path, names: &[&str]) {
    for name in names {
        let pid = pid_of(root, name);
        let deadline = Instant::now() + Duration::from_secs(10);
        while alive(pid) {
            assert!(
                Instant::now() < deadline,
                "{name} process {pid} survived containment"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

const GENERATIONS: [&str; 5] = ["root", "gen-3", "gen-2", "gen-1", "gen-0"];

/// An unrelated process in its own group. It must outlive every cleanup.
struct Sentinel(std::process::Child);

impl Sentinel {
    fn new() -> Self {
        let mut command = Command::new("/bin/sleep");
        command.arg("120").stdin(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Self(command.spawn().unwrap())
    }

    fn assert_alive(&mut self) {
        assert!(
            self.0.try_wait().unwrap().is_none(),
            "an unrelated sentinel process was terminated"
        );
    }
}

impl Drop for Sentinel {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn rooted(isolated: &Isolated, mode: &str, name: &str) -> (ProcessSpec, PathBuf) {
    let root = isolated.root.join(name);
    std::fs::create_dir_all(&root).unwrap();
    let mut spec = isolated.spec(mode);
    spec.command.push(root.to_string_lossy().into_owned());
    (spec, root)
}

fn macos(identity: &ContainmentIdentity) -> (u32, u32, PathBuf) {
    let ContainmentIdentity::MacosGuard {
        guard_pid,
        pgid,
        receipt,
        ..
    } = identity
    else {
        panic!("wrong platform identity");
    };
    (*guard_pid, *pgid, receipt.clone())
}

#[tokio::test]
async fn identity_names_every_required_owner_and_no_clock() {
    let isolated = Isolated::new();
    let (_keepalive, cancel) = oneshot::channel();
    let output = execute(&isolated.spec("emit"), Vec::new(), cancel, |_| async {
        Ok(())
    })
    .await
    .unwrap();
    let value = serde_json::to_value(&output.containment).unwrap();
    assert_eq!(value["platform"], "macos-guard");
    assert_eq!(value["scope"], "assigned-process-group");
    assert_eq!(value["anchor"], "unreaped-direct-child-leader");
    assert_eq!(value["leader_pid"], value["pgid"]);
    assert_ne!(value["guard_pid"], value["leader_pid"]);
    assert!(uuid::Uuid::parse_str(value["boot_session"].as_str().unwrap()).is_ok());
    for key in [
        "guard_started_us",
        "leader_started_us",
        "nonce",
        "receipt",
        "version",
    ] {
        assert!(!value[key].is_null(), "{key}");
    }
    // Boot identity is the session UUID; no clock-derived boot time exists.
    assert!(value
        .as_object()
        .unwrap()
        .keys()
        .all(|key| !key.contains("boottime")));
    // Finalization leaves exactly the receipt, never a partial file.
    let (_, _, receipt) = macos(&output.containment);
    let names: Vec<_> = std::fs::read_dir(receipt.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![receipt.file_name().unwrap().to_owned()]);
}

#[tokio::test]
async fn generations_by_fork_spawn_and_vfork_end_on_timeout_and_sentinel_survives() {
    let isolated = Isolated::new();
    let mut sentinel = Sentinel::new();
    let (mut spec, root) = rooted(&isolated, "gens", "timeout");
    spec.timeout = Duration::from_secs(3);
    let (_keepalive, cancel) = oneshot::channel();
    let error = execute(&spec, b"{}\n".to_vec(), cancel, |_| async { Ok(()) })
        .await
        .unwrap_err();
    assert_eq!(error.kind, ProcessFailureKind::Timeout);
    assert_ended(&root, &GENERATIONS);
    assert_eq!(
        probe_previous(error.containment.as_ref().unwrap()).await,
        ContainmentStatus::Ended
    );
    sentinel.assert_alive();
}

#[tokio::test]
async fn leader_exit_before_multi_generation_pipe_holding_descendants() {
    let isolated = Isolated::new();
    let mut sentinel = Sentinel::new();
    let (spec, root) = rooted(&isolated, "gens-return", "return");
    let (_keepalive, cancel) = oneshot::channel();
    let output = execute(&spec, b"{}\n".to_vec(), cancel, |_| async { Ok(()) })
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("complete"));
    assert_ended(&root, &GENERATIONS);
    assert_eq!(
        probe_previous(&output.containment).await,
        ContainmentStatus::Ended
    );
    sentinel.assert_alive();
}

#[tokio::test]
async fn exec_churn_overlapping_cleanup_ends_every_member() {
    let isolated = Isolated::new();
    let mut sentinel = Sentinel::new();
    for round in 0..5 {
        let (mut spec, root) = rooted(&isolated, "churn", &format!("churn-{round}"));
        spec.timeout = Duration::from_millis(1500);
        let (_keepalive, cancel) = oneshot::channel();
        let error = execute(&spec, b"{}\n".to_vec(), cancel, |_| async { Ok(()) })
            .await
            .unwrap_err();
        assert_eq!(error.kind, ProcessFailureKind::Timeout, "{error}");
        assert_ended(&root, &GENERATIONS);
        assert_ended(&root, &["reexec-0", "reexec-1", "reexec-2", "reexec-3"]);
        assert_eq!(
            probe_previous(error.containment.as_ref().unwrap()).await,
            ContainmentStatus::Ended
        );
    }
    sentinel.assert_alive();
}

#[tokio::test]
async fn detached_pipe_holder_is_bounded_and_never_claimed() {
    let isolated = Isolated::new();
    let (mut spec, root) = rooted(&isolated, "detach", "detach");
    // The escaped process holds the output pipes for eight seconds.
    spec.command.push("8".into());
    spec.timeout = Duration::from_secs(30);
    let (_keepalive, cancel) = oneshot::channel();
    let started = Instant::now();
    let error = execute(&spec, b"{}\n".to_vec(), cancel, |_| async { Ok(()) })
        .await
        .unwrap_err();
    assert_eq!(error.kind, ProcessFailureKind::Io, "{error}");
    assert!(error.message.contains("nonconforming"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(7));
    // The assigned group ended; the receipt says nothing about the process
    // that left it, and nothing signalled that process.
    assert_ended(&root, &["root"]);
    assert_eq!(
        probe_previous(error.containment.as_ref().unwrap()).await,
        ContainmentStatus::Ended
    );
    assert!(alive(pid_of(&root, "escaped")));
}

#[tokio::test]
async fn guard_loss_after_release_is_unproven_until_a_matching_late_receipt() {
    let isolated = Isolated::new();
    let (mut spec, root) = rooted(&isolated, "linger", "linger");
    // Finite lifetime: nothing may kill the detector once ownership is lost.
    spec.command.push("3".into());
    spec.timeout = Duration::from_secs(30);
    let recorded = Arc::new(Mutex::new(None));
    let (_keepalive, cancel) = oneshot::channel();
    let slot = recorded.clone();
    let attempt = execute(
        &spec,
        b"{}\n".to_vec(),
        cancel,
        move |identity| async move {
            *slot.lock().unwrap() = Some(identity);
            Ok(())
        },
    );
    let kill_guard = async {
        wait_file_async(&root.join("root.pid")).await;
        let identity = recorded.lock().unwrap().clone().unwrap();
        let (guard_pid, _, _) = macos(&identity);
        // The guard is this test process's own direct child.
        assert_eq!(unsafe { libc::kill(guard_pid as i32, libc::SIGKILL) }, 0);
        identity
    };
    let (result, identity) = tokio::join!(attempt, kill_guard);
    let error = result.unwrap_err();
    assert_eq!(error.kind, ProcessFailureKind::ContainmentUnproven);
    assert_eq!(error.containment.as_deref(), Some(&identity));
    assert_eq!(probe_previous(&identity).await, ContainmentStatus::Unproven);

    let (_, _, receipt) = macos(&identity);
    let proof = serde_json::json!({
        "version": 1, "identity": identity, "ended": true,
        "exit_code": null, "spawn_error": null
    });
    // Truncated, partial-only and mismatched evidence stays unproven.
    let bytes = serde_json::to_vec(&proof).unwrap();
    std::fs::write(&receipt, &bytes[..bytes.len() / 2]).unwrap();
    assert_eq!(probe_previous(&identity).await, ContainmentStatus::Unproven);
    std::fs::remove_file(&receipt).unwrap();
    let mut other = proof.clone();
    other["identity"]["nonce"] = "another-attempt".into();
    std::fs::write(&receipt, serde_json::to_vec(&other).unwrap()).unwrap();
    assert_eq!(probe_previous(&identity).await, ContainmentStatus::Unproven);
    std::fs::remove_file(&receipt).unwrap();
    let mut unfinished = proof.clone();
    unfinished["ended"] = false.into();
    std::fs::write(&receipt, serde_json::to_vec(&unfinished).unwrap()).unwrap();
    assert_eq!(probe_previous(&identity).await, ContainmentStatus::Unproven);
    std::fs::remove_file(&receipt).unwrap();
    // Resume accepts an exact, finalized, identity-matched receipt that
    // arrives after the runtime already reported Unproven.
    std::fs::write(&receipt, &bytes).unwrap();
    assert_eq!(probe_previous(&identity).await, ContainmentStatus::Ended);
    assert_ended(&root, &["root"]);
}

#[tokio::test]
async fn boot_session_change_clears_containment_and_the_same_boot_does_not() {
    let isolated = Isolated::new();
    let (_keepalive, cancel) = oneshot::channel();
    let output = execute(&isolated.spec("emit"), Vec::new(), cancel, |_| async {
        Ok(())
    })
    .await
    .unwrap();
    let mut value = serde_json::to_value(&output.containment).unwrap();
    value["nonce"] = "no-such-attempt".into();
    value["receipt"] = isolated
        .root
        .join("containment/no-such-attempt.ended.json")
        .to_string_lossy()
        .into_owned()
        .into();
    let same_boot: ContainmentIdentity = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(
        probe_previous(&same_boot).await,
        ContainmentStatus::Unproven
    );
    value["boot_session"] = uuid::Uuid::new_v4().to_string().to_uppercase().into();
    let other_boot: ContainmentIdentity = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(probe_previous(&other_boot).await, ContainmentStatus::Ended);
    // Unknown versions, scopes and malformed boot identities never clear.
    for (key, replacement) in [
        ("version", serde_json::json!(2)),
        ("scope", serde_json::json!("process-tree")),
        ("boot_session", serde_json::json!("")),
    ] {
        let mut changed = value.clone();
        changed[key] = replacement;
        let identity: ContainmentIdentity = serde_json::from_value(changed).unwrap();
        assert_eq!(
            probe_previous(&identity).await,
            ContainmentStatus::Unproven,
            "{key}"
        );
    }
}

/// Speak the private guard protocol directly so the test controls exactly
/// what the guard inherits.
struct DrivenGuard {
    child: std::process::Child,
    control: UnixStream,
    receipt: PathBuf,
    handshake: serde_json::Value,
}

fn drive_guard(
    isolated: &Isolated,
    executable: &Path,
    command: Vec<String>,
    inherit: Option<i32>,
) -> DrivenGuard {
    let containment = isolated.root.join("containment");
    std::fs::create_dir_all(&containment).unwrap();
    let nonce = uuid::Uuid::new_v4().to_string();
    let receipt = std::fs::canonicalize(&containment)
        .unwrap()
        .join(format!("{nonce}.ended.json"));
    let (mut control, guard_control) = UnixStream::pair().unwrap();
    control
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let fd = guard_control.as_raw_fd();
    let mut spawn = Command::new(executable);
    spawn
        .arg(fd.to_string())
        .current_dir(&isolated.root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    unsafe {
        spawn.pre_exec(move || {
            // A hostile inheritance: ignored SIGCHLD would auto-reap the
            // leader, and a stray descriptor could mask runtime death.
            libc::signal(libc::SIGCHLD, libc::SIG_IGN);
            if libc::setsid() < 0 || libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if let Some(extra) = inherit {
                if libc::fcntl(extra, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let child = spawn.spawn().unwrap();
    drop(guard_control);
    let launch = serde_json::json!({"command": command, "receipt": receipt, "nonce": nonce});
    control
        .write_all(&serde_json::to_vec(&launch).unwrap())
        .unwrap();
    control.write_all(b"\n").unwrap();
    let mut line = Vec::new();
    BufReader::new(control.try_clone().unwrap())
        .take(16 * 1024)
        .read_until(b'\n', &mut line)
        .unwrap();
    DrivenGuard {
        child,
        control,
        receipt,
        handshake: serde_json::from_slice(&line).unwrap(),
    }
}

#[test]
fn inherited_sigchld_ignore_and_stray_descriptors_do_not_reach_the_attempt() {
    let isolated = Isolated::new();
    let root = isolated.root.join("driven");
    std::fs::create_dir_all(&root).unwrap();
    let mut pipe = [0; 2];
    assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
    let (leak_read, leak_write) = (pipe[0], pipe[1]);
    let mut driven = drive_guard(
        &isolated,
        &guard(),
        vec![
            fixture().to_string_lossy().into_owned(),
            "linger".into(),
            root.to_string_lossy().into_owned(),
            "3".into(),
        ],
        Some(leak_write),
    );
    assert_eq!(
        driven.handshake["handshake"], "ready",
        "{}",
        driven.handshake
    );
    driven.control.write_all(b"S").unwrap();
    wait_file(&root.join("root.pid"));
    // Only the guard and the detector could still hold the stray write end.
    // EOF while the detector is alive proves both closed it at startup.
    unsafe { libc::close(leak_write) };
    let started = Instant::now();
    let mut reader = unsafe { <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(leak_read) };
    let mut byte = [0];
    assert_eq!(reader.read(&mut byte).unwrap(), 0);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(alive(pid_of(&root, "root")));
    // With SIGCHLD ignored at exec the leader would have been auto-reaped and
    // its exit code lost; the guard restored the default before forking.
    assert!(driven.child.wait().unwrap().success());
    let proof: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&driven.receipt).unwrap()).unwrap();
    assert_eq!(proof["ended"], true);
    assert_eq!(proof["exit_code"], 0);
    assert_eq!(proof["identity"], driven.handshake["identity"]);
}

#[test]
fn runtime_loss_before_release_never_executes_and_still_proves_the_group_ended() {
    let isolated = Isolated::new();
    let marker = isolated.root.join("must-not-execute");
    let mut driven = drive_guard(
        &isolated,
        &guard(),
        vec![
            fixture().to_string_lossy().into_owned(),
            "marker".into(),
            marker.to_string_lossy().into_owned(),
        ],
        None,
    );
    assert_eq!(driven.handshake["handshake"], "ready");
    // The runtime disappears with the gate still closed.
    driven.control.shutdown(std::net::Shutdown::Both).unwrap();
    assert!(driven.child.wait().unwrap().success());
    assert!(!marker.exists());
    let proof: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&driven.receipt).unwrap()).unwrap();
    assert_eq!(proof["ended"], true);
    assert!(proof["exit_code"].is_null());
}

/// A translated guard must refuse; set `WATCHER_TEST_X86_64_GUARD` to an
/// x86_64 build of the guard on Apple Silicon with Rosetta installed.
#[test]
fn translated_guard_is_refused_before_launch() {
    let Some(translated) = std::env::var_os("WATCHER_TEST_X86_64_GUARD") else {
        eprintln!("NOT COVERED: no x86_64 guard supplied; translation refusal not exercised");
        return;
    };
    let isolated = Isolated::new();
    let marker = isolated.root.join("must-not-execute");
    let mut driven = drive_guard(
        &isolated,
        Path::new(&translated),
        vec![
            fixture().to_string_lossy().into_owned(),
            "marker".into(),
            marker.to_string_lossy().into_owned(),
        ],
        None,
    );
    assert_eq!(
        driven.handshake["handshake"], "refused",
        "{}",
        driven.handshake
    );
    assert_eq!(driven.handshake["admission"], true);
    assert!(!driven.child.wait().unwrap().success());
    assert!(!marker.exists() && !driven.receipt.exists());
}

/// A translated detector under a native guard is an ordinary group member;
/// set `WATCHER_TEST_X86_64_FIXTURE` to an x86_64 build of the fixture.
#[tokio::test]
async fn translated_detector_descendant_is_an_ordinary_member() {
    let Some(translated) = std::env::var_os("WATCHER_TEST_X86_64_FIXTURE") else {
        eprintln!("NOT COVERED: no x86_64 fixture supplied; translated detector not exercised");
        return;
    };
    let isolated = Isolated::new();
    let (mut spec, root) = rooted(&isolated, "tree", "translated");
    spec.command[0] = PathBuf::from(translated).to_string_lossy().into_owned();
    spec.timeout = Duration::from_secs(3);
    let (_keepalive, cancel) = oneshot::channel();
    let error = execute(&spec, b"{}\n".to_vec(), cancel, |_| async { Ok(()) })
        .await
        .unwrap_err();
    assert_eq!(error.kind, ProcessFailureKind::Timeout, "{error}");
    assert_tree_ended(&root);
    assert_eq!(
        probe_previous(error.containment.as_ref().unwrap()).await,
        ContainmentStatus::Ended
    );
}

#[test]
fn concurrent_attempts_all_end_after_abrupt_runtime_death() {
    let isolated = Isolated::new();
    let mut sentinel = Sentinel::new();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "macos_conformance::fixture_runtime_concurrent",
            "--ignored",
            "--nocapture",
        ])
        .env("WATCHER_FIXTURE_ROOT", &isolated.root)
        .env("WATCHER_FIXTURE_BIN", fixture())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    let mut runtime = RuntimeChild(command.spawn().unwrap());
    let roots: Vec<PathBuf> = (0..4)
        .map(|index| isolated.root.join(format!("a{index}")))
        .collect();
    for root in &roots {
        wait_file(&root.join("gen-0.pid"));
        wait_file(&root.join("identity.json"));
    }
    let identities: Vec<_> = roots.iter().map(|root| read_identity(root)).collect();
    runtime.kill().unwrap();
    runtime.wait().unwrap();
    // Each guard observes runtime death on its own liveness handle; none is
    // kept alive by a handle another attempt inherited.
    for root in &roots {
        assert_ended(root, &GENERATIONS);
    }
    let probe = tokio::runtime::Runtime::new().unwrap();
    for identity in &identities {
        assert_eq!(
            probe.block_on(probe_previous(identity)),
            ContainmentStatus::Ended
        );
    }
    sentinel.assert_alive();
}

#[test]
#[ignore = "invoked only as an isolated subprocess by destructive containment tests"]
fn fixture_runtime_concurrent() {
    let base = PathBuf::from(std::env::var_os("WATCHER_FIXTURE_ROOT").unwrap());
    let binary = PathBuf::from(std::env::var_os("WATCHER_FIXTURE_BIN").unwrap());
    assert!(base.is_absolute() && binary.is_absolute());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let mut attempts = Vec::new();
        for index in 0..4 {
            let root = base.join(format!("a{index}"));
            std::fs::create_dir_all(&root).unwrap();
            let binary = binary.clone();
            attempts.push(tokio::spawn(async move {
                let spec = ProcessSpec {
                    command: vec![
                        binary.to_string_lossy().into_owned(),
                        "gens".into(),
                        root.to_string_lossy().into_owned(),
                    ],
                    working_directory: root.clone(),
                    environment_allowlist: vec![
                        "TELEX_HOME".into(),
                        "TELEX_DB".into(),
                        "TELEX_INSTALL_ROOT".into(),
                    ],
                    timeout: Duration::from_secs(60),
                    containment_directory: root.join("containment"),
                };
                let (_keepalive, cancel) = oneshot::channel();
                let result = execute(&spec, b"{}\n".to_vec(), cancel, |identity| async move {
                    let mut file =
                        std::fs::File::create(root.join("identity.json.partial")).unwrap();
                    file.write_all(&serde_json::to_vec(&identity).unwrap())
                        .unwrap();
                    file.sync_all().unwrap();
                    std::fs::rename(
                        root.join("identity.json.partial"),
                        root.join("identity.json"),
                    )
                    .unwrap();
                    Ok(())
                })
                .await;
                panic!("fixture runtime unexpectedly completed: {result:?}");
            }));
        }
        for attempt in attempts {
            let _ = attempt.await;
        }
    });
}

const PROC_PGRP_ONLY: u32 = 2;

fn group_census(pgid: i32) -> (i32, [i32; 2]) {
    let mut pids = [0i32; 2];
    let bytes = unsafe {
        libc::proc_listpids(
            PROC_PGRP_ONLY,
            pgid as u32,
            pids.as_mut_ptr().cast(),
            std::mem::size_of_val(&pids) as i32,
        )
    };
    (bytes, pids)
}

/// Negative control. One successful group SIGKILL is not completion: the
/// census can still see members right after the signal returns. The control
/// is preserved as a failing method, never relabelled as a passing one.
#[test]
fn one_shot_group_kill_is_not_a_completion_predicate() {
    let isolated = Isolated::new();
    let mut observed_members_after_one_shot = 0;
    const TRIALS: usize = 12;
    for trial in 0..TRIALS {
        let root = isolated.root.join(format!("one-shot-{trial}"));
        std::fs::create_dir_all(&root).unwrap();
        let mut command = Command::new(fixture());
        command
            .args(["churn", &root.to_string_lossy()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        // This test is the anchor owner: the leader is its unreaped child.
        let mut leader = command.spawn().unwrap();
        let pgid = leader.id() as i32;
        wait_file(&root.join("gen-0.pid"));
        for index in 0..4 {
            wait_file(&root.join(format!("reexec-{index}.pid")));
        }
        assert_eq!(unsafe { libc::kill(-pgid, libc::SIGKILL) }, 0);
        let (bytes, pids) = group_census(pgid);
        if !(bytes == 4 && pids[0] == pgid) {
            observed_members_after_one_shot += 1;
        }
        // The shipped predicate: repeat signal + census while anchored.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (bytes, pids) = group_census(pgid);
            if bytes == 4 && pids[0] == pgid {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "group never reduced to its leader"
            );
            unsafe { libc::kill(-pgid, libc::SIGKILL) };
            std::thread::sleep(Duration::from_millis(2));
        }
        leader.wait().unwrap();
        assert_eq!(unsafe { libc::kill(-pgid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }
    if observed_members_after_one_shot == 0 {
        eprintln!("INCONCLUSIVE: no member was observed after a one-shot kill in {TRIALS} trials");
    } else {
        eprintln!(
            "negative control preserved: {observed_members_after_one_shot}/{TRIALS} one-shot kills left visible members"
        );
    }
}

/// Bounded PID/PGID reuse. After Ended, an unrelated process that receives
/// the former leader's PID (and makes it a PGID) must never be signalled, and
/// recovery must still rely only on the receipt. Failing to observe reuse is
/// inconclusive, not success.
#[test]
#[ignore = "forks until the PID space wraps; run explicitly as conformance evidence"]
fn reused_pid_and_pgid_are_never_signalled_after_ended() {
    let isolated = Isolated::new();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let attempt = |isolated: &Isolated| {
        let (_keepalive, cancel) = oneshot::channel();
        runtime
            .block_on(execute(
                &isolated.spec("emit"),
                Vec::new(),
                cancel,
                |_| async { Ok(()) },
            ))
            .unwrap()
            .containment
    };
    let identity = attempt(&isolated);
    let (_, former, _) = macos(&identity);
    let former = former as i32;
    let deadline = Instant::now() + Duration::from_secs(240);
    let mut forks = 0u64;
    let mut reused = None;
    while Instant::now() < deadline && forks < 400_000 {
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0);
        if pid == 0 {
            unsafe {
                if libc::getpid() == former {
                    // Become a group whose ID is the former PGID, then wait.
                    libc::setpgid(0, 0);
                    libc::sleep(60);
                }
                libc::_exit(0);
            }
        }
        forks += 1;
        if pid == former {
            reused = Some(pid);
            break;
        }
        let mut status = 0;
        assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
    }
    let Some(sentinel) = reused else {
        eprintln!("INCONCLUSIVE: PID {former} was not reused within {forks} forks");
        return;
    };
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(unsafe { libc::getpgid(sentinel) }, former);
    // Recovery of the old attempt and several new attempts all proceed
    // while an unrelated group owns the old number.
    assert_eq!(
        runtime.block_on(probe_previous(&identity)),
        ContainmentStatus::Ended
    );
    for _ in 0..3 {
        let next = attempt(&isolated);
        assert_ne!(macos(&next).1 as i32, former);
    }
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(sentinel, &mut status, libc::WNOHANG) },
        0,
        "the unrelated owner of a reused PID/PGID was terminated"
    );
    eprintln!("reuse observed after {forks} forks; the new owner of PID/PGID {former} survived");
    unsafe {
        // Our own unreaped direct child.
        libc::kill(sentinel, libc::SIGKILL);
        libc::waitpid(sentinel, &mut status, 0);
    }
}
