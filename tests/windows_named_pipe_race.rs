//! Bounded hosted-only diagnostic for the Tokio named-pipe I/O lifetime path that the
//! daemon transport uses (`daemon.rs` Windows client and server pipes).
//!
//! Each round opens a byte-mode duplex pipe with the daemon's 8192-byte buffers. Both
//! ends run a reader and a writer with randomized payload sizes, and one randomly chosen
//! end is aborted mid-flight (pending futures cancelled, handle dropped) after a random
//! number of yields. It is a seeded randomized race loop with a fixed bound, not a
//! deterministic trigger: a clean run does not prove the absence of the defect. It only
//! executes on disposable GitHub-hosted runners, under PageHeap with postmortem capture,
//! via `.github/workflows/ci.yml` (`windows-named-pipe-race`).
#![cfg(windows)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};
use tokio::task::JoinSet;

static NEXT_PIPE: AtomicU64 = AtomicU64::new(0);
const DAEMON_PIPE_BUFFER: u32 = 8192;

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

struct EndPlan {
    write_len: usize,
    read_len: usize,
}

fn spawn_end<S>(
    stream: S,
    plan: EndPlan,
) -> (tokio::task::JoinHandle<()>, tokio::task::JoinHandle<()>)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (mut read_half, mut write_half) = tokio::io::split(stream);
    let reader = tokio::spawn(async move {
        let mut buffer = vec![0u8; plan.read_len];
        loop {
            match read_half.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    });
    let writer = tokio::spawn(async move {
        let payload = vec![0xA5u8; plan.write_len];
        let _ = write_half.write_all(&payload).await;
        let _ = write_half.shutdown().await;
    });
    (reader, writer)
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
        .in_buffer_size(DAEMON_PIPE_BUFFER)
        .out_buffer_size(DAEMON_PIPE_BUFFER)
        .create(&name)
        .expect("create the hosted race pipe");
    let client = ClientOptions::new()
        .open(&name)
        .expect("open the hosted race pipe client");
    server
        .connect()
        .await
        .expect("connect the hosted race pipe");

    let plan = |random: &mut u64| EndPlan {
        write_len: match next_random(random) % 4 {
            0 => 1 + (next_random(random) % 64) as usize,
            1 => 1 + (next_random(random) % 8192) as usize,
            2 => 8192 + (next_random(random) % 65536) as usize,
            _ => 1 + (next_random(random) % (512 * 1024)) as usize,
        },
        read_len: 1 + (next_random(random) % 9000) as usize,
    };
    let server_plan = plan(&mut random);
    let client_plan = plan(&mut random);
    let abort_server = next_random(&mut random).is_multiple_of(2);
    let yields = next_random(&mut random) % 16;
    let pause_micros = next_random(&mut random) % 3 * 50;

    let (server_reader, server_writer) = spawn_end(server, server_plan);
    let (client_reader, client_writer) = spawn_end(client, client_plan);
    for _ in 0..yields {
        tokio::task::yield_now().await;
    }
    if pause_micros > 0 {
        tokio::time::sleep(Duration::from_micros(pause_micros)).await;
    }

    let victim = if abort_server {
        [&server_reader, &server_writer]
    } else {
        [&client_reader, &client_writer]
    };
    for task in victim {
        task.abort();
    }
    for (label, task) in [
        ("server reader", server_reader),
        ("server writer", server_writer),
        ("client reader", client_reader),
        ("client writer", client_writer),
    ] {
        match tokio::time::timeout(Duration::from_secs(60), task).await {
            Err(_) => panic!("hosted race {label} did not finish for seed {seed:#x}"),
            Ok(Err(error)) if !error.is_cancelled() => {
                panic!("hosted race {label} failed for seed {seed:#x}: {error}")
            }
            Ok(_) => {}
        }
    }
}

#[test]
#[ignore = "explicit disposable GitHub-hosted native diagnostic"]
fn abrupt_close_races_pipe_reads_and_writes() {
    assert!(
        std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            && std::env::var("RUNNER_ENVIRONMENT").as_deref() == Ok("github-hosted"),
        "the named-pipe race diagnostic is restricted to disposable GitHub-hosted runners"
    );
    let rounds = env_u64("TELEX_PIPE_RACE_ROUNDS", 20_000);
    let concurrency = env_u64("TELEX_PIPE_RACE_CONCURRENCY", 32);
    let workers = env_u64("TELEX_PIPE_RACE_WORKERS", 4) as usize;
    let seed = env_u64("TELEX_PIPE_RACE_SEED", 0x166_0000_0001);
    let max_seconds = env_u64("TELEX_PIPE_RACE_MAX_SECONDS", 600);
    println!(
        "named-pipe race: rounds={rounds} concurrency={concurrency} workers={workers} \
         seed={seed:#x} max_seconds={max_seconds}"
    );

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
        .expect("build the multi-thread runtime");
    let started_at = Instant::now();
    let finished = runtime.block_on(async move {
        let mut random = seed;
        let mut tasks = JoinSet::new();
        let mut started = 0u64;
        let mut finished = 0u64;
        while finished < rounds && started_at.elapsed().as_secs() < max_seconds {
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
            if finished.is_multiple_of(10_000) {
                println!("named-pipe race: completed {finished}/{rounds}");
            }
        }
        while let Some(result) = tasks.join_next().await {
            result.expect("a hosted race round completed");
            finished += 1;
        }
        finished
    });
    println!(
        "named-pipe race: completed {finished} rounds in {}s",
        started_at.elapsed().as_secs()
    );
}
