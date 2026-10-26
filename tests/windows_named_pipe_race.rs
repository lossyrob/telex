//! Bounded hosted-only diagnostic for the Tokio named-pipe I/O lifetime path that the
//! daemon transport uses (`daemon.rs` Windows client and server pipes).
//!
//! The workload closes the client end abruptly while server reads and large writes are
//! being submitted from different runtime workers. It is a randomized race loop with a
//! fixed seed and iteration bound, not a deterministic trigger: a clean run does not
//! prove the absence of the defect. It only executes on disposable GitHub-hosted
//! runners, under PageHeap with postmortem capture, via
//! `.github/workflows/ci.yml` (`windows-named-pipe-race`).
#![cfg(windows)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};
use tokio::task::JoinSet;

static NEXT_PIPE: AtomicU64 = AtomicU64::new(0);

fn next_random(state: &mut u64) -> u64 {
    // SplitMix64: deterministic for a given seed so a failing sequence can be compared.
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

async fn one_round(seed: u64) {
    let mut random = seed;
    let name = format!(
        r"\\.\pipe\telex-hosted-race-{}-{}",
        std::process::id(),
        NEXT_PIPE.fetch_add(1, Ordering::Relaxed)
    );
    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&name)
        .expect("create the hosted race pipe");
    let client = ClientOptions::new()
        .open(&name)
        .expect("open the hosted race pipe client");
    server
        .connect()
        .await
        .expect("connect the hosted race pipe");

    let (mut server_read, mut server_write) = tokio::io::split(server);
    let write_len = 1 + (next_random(&mut random) % (512 * 1024)) as usize;
    let client_write_len = (next_random(&mut random) % (64 * 1024)) as usize;
    let client_yields = next_random(&mut random) % 8;
    let client_reads_first = next_random(&mut random) % 2 == 0;

    let reader = tokio::spawn(async move {
        let mut buffer = vec![0u8; 1 + (seed % 4096) as usize];
        loop {
            match server_read.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    });
    let writer = tokio::spawn(async move {
        let payload = vec![0xA5u8; write_len];
        let _ = server_write.write_all(&payload).await;
        let _ = server_write.shutdown().await;
    });
    let closer = tokio::spawn(async move {
        let mut client = client;
        if client_reads_first {
            let mut sink = [0u8; 512];
            let _ = client.read(&mut sink).await;
        }
        let payload = vec![0x5Au8; client_write_len];
        let _ = client.write_all(&payload).await;
        for _ in 0..client_yields {
            tokio::task::yield_now().await;
        }
        drop(client);
    });

    for (label, task) in [("closer", closer), ("writer", writer), ("reader", reader)] {
        tokio::time::timeout(Duration::from_secs(60), task)
            .await
            .unwrap_or_else(|_| panic!("hosted race {label} task did not finish for seed {seed}"))
            .unwrap_or_else(|error| panic!("hosted race {label} task failed: {error}"));
    }
}

#[test]
#[ignore = "explicit disposable GitHub-hosted native diagnostic"]
fn abrupt_client_close_races_server_reads_and_writes() {
    assert!(
        std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && std::env::var("RUNNER_ENVIRONMENT").as_deref() == Ok("github-hosted"),
        "the named-pipe race diagnostic is restricted to disposable GitHub-hosted runners"
    );
    let rounds = env_u64("TELEX_PIPE_RACE_ROUNDS", 20_000);
    let concurrency = env_u64("TELEX_PIPE_RACE_CONCURRENCY", 32);
    let seed = env_u64("TELEX_PIPE_RACE_SEED", 0x166_0000_0001);
    println!("named-pipe race: rounds={rounds} concurrency={concurrency} seed={seed:#x}");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("build the multi-thread runtime");
    runtime.block_on(async move {
        let mut random = seed;
        let mut tasks = JoinSet::new();
        let mut started = 0u64;
        let mut finished = 0u64;
        while finished < rounds {
            while started < rounds && tasks.len() < concurrency as usize {
                tasks.spawn(one_round(next_random(&mut random)));
                started += 1;
            }
            tasks
                .join_next()
                .await
                .expect("a round is in flight")
                .expect("a hosted race round completed");
            finished += 1;
            if finished % 2_000 == 0 {
                println!("named-pipe race: completed {finished}/{rounds}");
            }
        }
    });
    println!("named-pipe race: completed all {rounds} rounds");
}
