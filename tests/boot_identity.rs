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

use std::process::Command;

/// Marker the child looks for. Set only by the parent test below.
const EMIT_ENV: &str = "TELEX_TEST_EMIT_BOOT_IDENTITY";
const CACHED_PREFIX: &str = "TELEX_BOOT_ID_CACHED=";
const UNCACHED_PREFIX: &str = "TELEX_BOOT_ID_UNCACHED=";
const HOST_PREFIX: &str = "TELEX_HOST_ID=";

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
