//! Boot-session identity, asserted the only way that means anything: across a genuinely
//! independent process.
//!
//! `platform_fs::boot_id()` is compared for **exact equality** between two separate processes — the
//! attaching CLI writes it into a station intent, the daemon recomputes it and refuses anything
//! that does not match. A disagreement is not a degradation: every intent terminates as
//! `foreign_host_or_boot`, GC then removes it as a foreign identity with a dead producer, and the
//! anti-downgrade guard turns the same condition into a hard refusal of an unrelated
//! `telex attach`.
//!
//! An in-process test cannot see that failure mode at all. `boot_id()` memoizes in a `OnceLock`, so
//! a loop over it compares a cached `String` to itself; and even the uncached resolver, called
//! repeatedly in one process, shares that process's registry handle behaviour and its
//! `GetTickCount64` sampling. Only a second process closes the gap, and on Windows only a second
//! process actually exercises the persist-then-read-back path that this identity depends on.
//!
//! The child is this test binary, re-invoked with `--exact` on the emitter below. That keeps the
//! test hermetic (no build-order dependency on the `telex` binary, no new CLI surface minted purely
//! for a test) while still being a real, separate process with its own address space, its own
//! `OnceLock`, and its own view of the registry.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Marker the child looks for. Set only by the parent test below.
const EMIT_ENV: &str = "TELEX_TEST_EMIT_BOOT_IDENTITY";
const CACHED_PREFIX: &str = "TELEX_BOOT_ID_CACHED=";
const UNCACHED_PREFIX: &str = "TELEX_BOOT_ID_UNCACHED=";
const HOST_PREFIX: &str = "TELEX_HOST_ID=";

/// Isolation for the cold-start race: the storage namespace the child resolves in, and the
/// directory holding the barrier files. Both are per-run and are set only by the parent below,
/// so nothing in the normal suite — and nothing in the developer's real per-user boot record —
/// is touched.
const NAMESPACE_ENV: &str = "TELEX_TEST_BOOT_ID_NAMESPACE";
const BARRIER_ENV: &str = "TELEX_TEST_BOOT_ID_BARRIER";
const COLD_START_PREFIX: &str = "TELEX_BOOT_ID_COLD_START=";

/// Barrier file names. `ARRIVED_PREFIX` is written by each child once it has observed the record
/// missing; `RELEASE` is written by the parent once every child has.
const ARRIVED_PREFIX: &str = "arrived-";
const RELEASE: &str = "release";

/// The child half. Ordinarily a no-op; under `EMIT_ENV` it prints the identities and returns.
///
/// Deliberately not `#[ignore]`d: an ignored test does not run in the normal suite, and this one
/// must, so that a resolver that fails outright is caught here as well as in the child.
#[test]
fn boot_identity_emitter() {
    let host = telex::platform_fs::host_id().expect("host identity must resolve");
    let cached = telex::platform_fs::boot_id().expect("boot identity must resolve");
    let uncached = telex::platform_fs::boot_id_uncached().expect("boot identity must resolve");
    assert_eq!(
        cached, uncached,
        "the cached accessor must agree with the resolver it caches"
    );
    if std::env::var_os(EMIT_ENV).is_some() {
        println!("{HOST_PREFIX}{host}");
        println!("{CACHED_PREFIX}{cached}");
        println!("{UNCACHED_PREFIX}{uncached}");
    }
}

fn field(haystack: &str, prefix: &str) -> String {
    haystack
        .lines()
        .find_map(|line| line.trim().strip_prefix(prefix))
        .unwrap_or_else(|| panic!("the child did not emit {prefix}; output was:\n{haystack}"))
        .trim()
        .to_string()
}

/// Spawn an independent process and require it to agree, exactly.
///
/// On Windows this is the whole of the regression: the identity is minted once per boot and
/// persisted in `HKCU\Software\telex`, and if persistence or read-back fails the resolver now
/// reports an error instead of quietly handing back a per-process random value. Either way the two
/// processes agree or the test fails — a silent per-process value would show up here as a mismatch,
/// and a failed persist as a child that exits non-zero with the cause named.
#[test]
fn boot_identity_agrees_across_an_independent_process() {
    let parent_host = telex::platform_fs::host_id().expect("host identity");
    let parent_boot = telex::platform_fs::boot_id().expect("boot identity");
    assert_eq!(parent_boot.len(), 32);

    let exe = std::env::current_exe().expect("test binary path");
    let output = Command::new(&exe)
        .args(["--exact", "boot_identity_emitter", "--nocapture"])
        .env(EMIT_ENV, "1")
        .output()
        .expect("spawn an independent process");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "the child must resolve its identity or say why it could not.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    assert_eq!(
        field(&stdout, HOST_PREFIX),
        parent_host,
        "host identity must be stable across processes"
    );
    assert_eq!(
        field(&stdout, CACHED_PREFIX),
        parent_boot,
        "two independent processes must agree on the boot identity, or every station intent \
         written by one and read by the other terminates as `foreign_host_or_boot`"
    );
    assert_eq!(
        field(&stdout, UNCACHED_PREFIX),
        parent_boot,
        "the persisted record, not a per-process value, is what the second process must read back"
    );
}

/// A third process, started after the first two, must still agree — the case a per-boot mint gets
/// wrong if the persistence write lands but the record is not honoured on read-back, or if the
/// validity window (uptime monotonicity plus the derived boot instant) is too tight to survive
/// ordinary sampling skew between processes started seconds apart.
#[test]
fn boot_identity_survives_repeated_independent_mint_attempts() {
    let parent = telex::platform_fs::boot_id().expect("boot identity");
    let exe = std::env::current_exe().expect("test binary path");
    for round in 0..3 {
        let output = Command::new(&exe)
            .args(["--exact", "boot_identity_emitter", "--nocapture"])
            .env(EMIT_ENV, "1")
            .output()
            .expect("spawn an independent process");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(
            output.status.success(),
            "round {round}: the child must resolve its identity.\n{stdout}"
        );
        assert_eq!(
            field(&stdout, UNCACHED_PREFIX),
            parent,
            "round {round}: a later process must read the persisted identity back rather than \
             minting one of its own"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// The racing half of the cold-start test. Inert unless the parent hands it a namespace.
///
/// It parks at the barrier **inside** the resolver, at the instant it has read the record and found
/// it missing and has not yet taken the mint lock. That is what makes the race a fact rather than a
/// hope: a launch barrier alone only releases the processes together, and the first one is free to
/// finish its entire mint before the twelfth has read the key — after which the other eleven take
/// the uncontended already-a-record path and a resolver with no serialization at all passes. Here
/// no participant advances past the observation until every participant has made it.
#[test]
fn boot_identity_cold_start_emitter() {
    let Ok(namespace) = std::env::var(NAMESPACE_ENV) else {
        return;
    };
    let barrier = PathBuf::from(
        std::env::var(BARRIER_ENV).expect("the parent sets the barrier alongside the namespace"),
    );
    let arrive = || {
        std::fs::write(
            barrier.join(format!("{ARRIVED_PREFIX}{}", std::process::id())),
            b"arrived",
        )
        .expect("announce arrival at the barrier");
        // Spin rather than sleep: a poll interval is a head start, and a head start is exactly what
        // lets a broken first-writer look correct.
        let release = barrier.join(RELEASE);
        let deadline = Instant::now() + Duration::from_secs(120);
        while !release.exists() {
            assert!(
                Instant::now() < deadline,
                "the barrier at {} was never released",
                barrier.display()
            );
            std::hint::spin_loop();
        }
    };

    // On Windows the resolver drives the barrier, because only Windows has the observe/mint window
    // to synchronize. Elsewhere the identity comes from the kernel: there is no record to observe,
    // no mint to serialize, and nothing for the hook to be called at — so the child arrives on its
    // own behalf and the parent's release is a formality it passes straight through. That parity is
    // the point of running this test on every platform.
    #[cfg(windows)]
    let resolved =
        telex::platform_fs::boot_id_uncached_in_test_namespace_at_cold_start(&namespace, &arrive);
    #[cfg(not(windows))]
    let resolved = {
        arrive();
        telex::platform_fs::boot_id_uncached_in_test_namespace(&namespace)
    };

    let id =
        resolved.expect("a cold-start boot identity must resolve, or fail closed naming its cause");
    println!("{COLD_START_PREFIX}{id}");
}

/// Everything one cold-start run owns, torn down exactly once however the run ends.
///
/// A panic mid-race is the case that matters. Without this, an assertion failure leaves twelve
/// child processes parked on a barrier that will never be released (they spin for two minutes, on
/// twelve cores, while the rest of the suite runs), an orphaned registry namespace, and a temp
/// directory — and `cargo test` reports the failure long before any of it goes away. So the guard
/// releases the barrier first (parked children resolve and exit rather than spinning), then kills
/// and *reaps* whatever is still running, then removes its own namespace and root.
///
/// Scoped strictly to this run: the namespace is unique per run and the guard deletes only that
/// one, never the shared container. `cargo test` runs test binaries concurrently and developers run
/// several checkouts at once; a container-wide sweep would delete a namespace another run was
/// mid-race in and fail an unrelated suite for reasons invisible in its own output.
struct ColdStartRun {
    namespace: String,
    barrier: PathBuf,
    racers: Vec<Child>,
}

impl ColdStartRun {
    fn new(namespace: String, barrier: PathBuf) -> Self {
        Self {
            namespace,
            barrier,
            racers: Vec::new(),
        }
    }

    fn spawn(&mut self, exe: &Path) -> usize {
        let child = Command::new(exe)
            .args(["--exact", "boot_identity_cold_start_emitter", "--nocapture"])
            .env(NAMESPACE_ENV, &self.namespace)
            .env(BARRIER_ENV, &self.barrier)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn a racing process");
        self.racers.push(child);
        self.racers.len() - 1
    }

    /// Take a spawned child back out of the guard's custody in order to wait on it. Anything left
    /// behind — because an assertion tripped part way through collection — is still killed and
    /// reaped by `drop`.
    fn take(&mut self, index: usize) -> Child {
        self.racers.remove(index)
    }

    fn release(&self) {
        std::fs::write(self.barrier.join(RELEASE), b"go").expect("release the barrier");
    }

    fn arrived(&self) -> usize {
        std::fs::read_dir(&self.barrier)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .filter(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(ARRIVED_PREFIX)
                    })
                    .count()
            })
            .unwrap_or(0)
    }
}

impl Drop for ColdStartRun {
    fn drop(&mut self) {
        // Release first: a child parked at the barrier exits on its own after this, so the kill
        // below is only for one that is genuinely wedged.
        let _ = std::fs::write(self.barrier.join(RELEASE), b"go");
        for racer in &mut self.racers {
            let _ = racer.kill();
            let _ = racer.wait();
        }
        let _ = telex::platform_fs::clear_test_boot_id_namespace(&self.namespace);
        let _ = std::fs::remove_dir_all(&self.barrier);
    }
}

/// Several processes cold-starting at the same instant must resolve **one** identity.
///
/// This is the race the persisted record does not close on its own. On Windows the first mint of a
/// boot is a read-modify-write over `HKCU\Software\telex`, and `RegSetKeyValueW` is an
/// unconditional overwrite: with no serialization, every process that finds the key empty mints its
/// own value and stamps it over the others'. The read-back does not rescue it either — a writer can
/// read its own value back before the next writer overwrites the key — so the processes disagree,
/// and a station intent written by one is `foreign_host_or_boot` to the other for the rest of the
/// boot. `telex copilot attach` spawning a daemon and a watcher is exactly this shape.
///
/// The proof therefore has to be cross-process and simultaneous: threads share the resolver's
/// `OnceLock` and one process's registry handles. And it has to be *deterministic*, not merely
/// simultaneous — hence the barrier inside the resolver rather than around it. Every racer blocks
/// after observing an empty record and before taking the mint lock, so when they are released the
/// contended first mint is guaranteed to have happened, and the assertion is on the *set* of
/// answers: exactly one distinct value.
///
/// Isolation: the race runs in a per-run storage namespace, removed by `ColdStartRun` however the
/// test ends, so the developer's (or runner's) real per-user boot record is never touched and no
/// concurrently running test binary is disturbed. On Linux and macOS the identity comes from the
/// kernel, there is nothing to mint and nothing to isolate, and this test states that parity —
/// every process reads the same kernel value, so the single-answer assertion holds there by
/// construction.
#[test]
fn a_concurrent_cold_start_resolves_exactly_one_boot_identity() {
    const RACERS: usize = 12;

    let namespace = format!(
        "cold-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    );
    let barrier = Path::new(env!("CARGO_TARGET_TMPDIR")).join(&namespace);
    std::fs::create_dir_all(&barrier).expect("barrier root");
    let mut run = ColdStartRun::new(namespace.clone(), barrier);
    telex::platform_fs::clear_test_boot_id_namespace(&namespace)
        .expect("the race must start from a genuinely empty record");

    let exe = std::env::current_exe().expect("test binary path");
    for _ in 0..RACERS {
        run.spawn(&exe);
    }

    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let arrived = run.arrived();
        if arrived >= RACERS {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "only {arrived} of {RACERS} racers reached the barrier having observed an empty record"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    run.release();

    let mut identities = BTreeSet::new();
    for index in 0..RACERS {
        // Always index 0: each collected racer is removed from the guard's custody, so anything an
        // assertion below skips is still killed and reaped by `ColdStartRun::drop`.
        let output = run
            .take(0)
            .wait_with_output()
            .expect("collect a racing process");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(
            output.status.success(),
            "racer {index} must resolve its identity or say why it could not.\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        identities.insert(field(&stdout, COLD_START_PREFIX));
    }

    assert_eq!(
        identities.len(),
        1,
        "{RACERS} processes cold-started together and came away with {} different boot \
         identities ({identities:?}); the first mint of a boot is not atomic, so an intent written \
         by one of them is `foreign_host_or_boot` to the others",
        identities.len()
    );
    let agreed = identities.into_iter().next().expect("one identity");
    assert_eq!(
        agreed.len(),
        32,
        "the resolved identity must keep the 32-character contract, got {agreed:?}"
    );

    // The winner's record has to be the durable one: a process arriving after the race must read
    // it back rather than mint again. That distinguishes "they agreed" from "they agreed by
    // accident and the key is still empty". It finds a valid record, so it never reaches the
    // barrier hook at all — which is itself the assertion that the uncontended path skips the lock.
    let index = run.spawn(&exe);
    let latecomer = run
        .take(index)
        .wait_with_output()
        .expect("collect the latecomer");
    let stdout = String::from_utf8_lossy(&latecomer.stdout).into_owned();
    assert!(
        latecomer.status.success(),
        "the latecomer must resolve its identity.\n{stdout}"
    );
    assert_eq!(
        field(&stdout, COLD_START_PREFIX),
        agreed,
        "a process started after the race must read the persisted record back"
    );
}

/// A namespace is a test container, never a way to reach the production record.
#[test]
fn a_boot_identity_test_namespace_cannot_escape_its_container() {
    for hostile in ["", "..", "a\\b", "telex\\..", "has space", &"x".repeat(49)] {
        assert!(
            telex::platform_fs::boot_id_uncached_in_test_namespace(hostile).is_err(),
            "{hostile:?} must be refused as a boot-identity test namespace"
        );
        assert!(
            telex::platform_fs::clear_test_boot_id_namespace(hostile).is_err(),
            "{hostile:?} must be refused when clearing a boot-identity test namespace"
        );
    }
}
