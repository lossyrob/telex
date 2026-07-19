//! Experimental, provider-neutral Watcher runtime. This crate deliberately owns only trusted
//! local detector execution, receipt-gated state transitions, and the temporary Telex adapter.

mod adapter;
mod protocol;
mod registry;

use adapter::{CliTelexAdapter, LifecycleCoordinator, SendRequest, ShutdownSignal};
use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use fs2::FileExt;
use protocol::{
    hash_value, normalized_envelope, parse_result, send_metadata, DetectorRequest, Outcome,
    ScriptRequest, ValidatedResult, WatchRequest, MAX_STDERR_BYTES, MAX_STDOUT_BYTES,
};
use registry::{now_ms, script_digest, Registry, ScriptMode, Watch, WatchSpec, WatchStatus};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "telex-watcher",
    version,
    about = "Experimental trusted-local detector runner for Telex"
)]
struct Cli {
    /// SQLite registry path. Defaults below the current user's local Telex data directory.
    #[arg(long, env = "TELEX_WATCHER_REGISTRY")]
    registry: Option<PathBuf>,
    /// Telex executable used by the experimental adapter.
    #[arg(long, env = "TELEX_WATCHER_TELEX", default_value = "telex")]
    telex: PathBuf,
    #[command(subcommand)]
    command: WatcherCommand,
}

#[derive(Subcommand)]
enum WatcherCommand {
    /// Add a local watch definition from JSON.
    Add {
        #[arg(long)]
        file: PathBuf,
    },
    /// List persisted watches.
    List,
    /// Show one persisted watch.
    Show { watch_id: String },
    /// Pause an active watch without removing its sender membership.
    Pause { watch_id: String },
    /// Resume a paused watch.
    Resume { watch_id: String },
    /// Replace mutable watch settings from JSON.
    Update {
        watch_id: String,
        #[arg(long)]
        file: PathBuf,
    },
    /// Retain a watch and its provenance while stopping it permanently.
    Remove { watch_id: String },
    /// List bounded attempt diagnostics.
    Attempts {
        watch_id: String,
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// List accepted-event provenance.
    Events {
        watch_id: String,
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Run the experimental scheduler.
    Run {
        /// Run the selected due watches once, then gracefully detach.
        #[arg(long)]
        once: bool,
        /// Limit execution to one or more watch IDs.
        #[arg(long = "watch")]
        watches: Vec<String>,
        /// Upper bound on globally in-flight detector attempts.
        #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u8).range(1..=16))]
        concurrency: u8,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunSummary {
    runtime_session_id: String,
    watcher_pid: u32,
    runs: usize,
    detached: Vec<DetachOutcome>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DetachOutcome {
    sender: String,
    outcome: String,
}

struct RuntimeRecord<'a> {
    id: i64,
    session_id: &'a str,
    watcher_pid: u32,
}

#[derive(Debug)]
struct ProcessOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

type SharedLifecycle<A> = Arc<Mutex<LifecycleCoordinator<A>>>;

struct ShutdownListener(tokio::task::JoinHandle<()>);

impl Drop for ShutdownListener {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Aborts a background task when the scheduler returns, so the independent periodic reconcile never
/// outlives the runtime that spawned it.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub fn run() -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("create Watcher runtime")?
        .block_on(run_cli())
}

fn fresh_runtime_session_id() -> String {
    Uuid::new_v4().to_string()
}

fn start_shutdown_listener(
    shutdown: ShutdownSignal,
    runtime_session_id: String,
    watcher_pid: u32,
) -> ShutdownListener {
    ShutdownListener(tokio::spawn(async move {
        match wait_for_shutdown_signal().await {
            Ok(signal) => {
                lifecycle_log(
                    "runtime-signal",
                    &runtime_session_id,
                    watcher_pid,
                    json!({"signal": signal}),
                );
                shutdown.request();
            }
            Err(error) => lifecycle_log(
                "runtime-signal-error",
                &runtime_session_id,
                watcher_pid,
                json!({"error": error.to_string()}),
            ),
        }
    }))
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() -> Result<&'static str> {
    use tokio::signal::unix::{signal, SignalKind};

    let mut terminate = signal(SignalKind::terminate()).context("subscribe to SIGTERM")?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            result.context("wait for Ctrl+C")?;
            Ok("SIGINT")
        }
        _ = terminate.recv() => Ok("SIGTERM"),
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() -> Result<&'static str> {
    tokio::signal::ctrl_c().await.context("wait for Ctrl+C")?;
    Ok("SIGINT")
}

async fn run_cli() -> Result<()> {
    let cli = Cli::parse();
    let registry_path = cli.registry.unwrap_or_else(default_registry_path);
    match cli.command {
        WatcherCommand::Add { file } => {
            let spec = read_spec(&file)?;
            let watch = Registry::open(&registry_path)?.add(spec)?;
            print_json(&watch)?;
        }
        WatcherCommand::List => print_json(&Registry::open(&registry_path)?.list()?)?,
        WatcherCommand::Show { watch_id } => {
            let watch = Registry::open(&registry_path)?
                .get(&watch_id)?
                .ok_or_else(|| anyhow!("watch {watch_id:?} does not exist"))?;
            print_json(&watch)?;
        }
        WatcherCommand::Pause { watch_id } => print_json(
            &Registry::open(&registry_path)?.set_status(&watch_id, WatchStatus::Paused)?,
        )?,
        WatcherCommand::Resume { watch_id } => print_json(
            &Registry::open(&registry_path)?.set_status(&watch_id, WatchStatus::Active)?,
        )?,
        WatcherCommand::Update { watch_id, file } => {
            let spec = read_spec(&file)?;
            print_json(&Registry::open(&registry_path)?.update(&watch_id, spec)?)?
        }
        WatcherCommand::Remove { watch_id } => {
            print_json(&Registry::open(&registry_path)?.remove(&watch_id)?)?
        }
        WatcherCommand::Attempts { watch_id, limit } => {
            let registry = Registry::open(&registry_path)?;
            require_watch(&registry, &watch_id)?;
            print_json(&registry.attempts(&watch_id, limit)?)?;
        }
        WatcherCommand::Events { watch_id, limit } => {
            let registry = Registry::open(&registry_path)?;
            require_watch(&registry, &watch_id)?;
            print_json(&registry.events(&watch_id, limit)?)?;
        }
        WatcherCommand::Run {
            once,
            watches,
            concurrency,
        } => {
            let summary =
                run_scheduler(&registry_path, cli.telex, once, watches, concurrency).await?;
            print_json(&summary)?;
        }
    }
    Ok(())
}

fn default_registry_path() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("telex")
        .join("watcher-v1.sqlite")
}

fn read_spec(path: &Path) -> Result<WatchSpec> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("read watch definition {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("parse watch definition {} as JSON", path.display()))
}

fn require_watch(registry: &Registry, id: &str) -> Result<()> {
    if registry.get(id)?.is_none() {
        bail!("watch {id:?} does not exist");
    }
    Ok(())
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

async fn run_scheduler(
    registry_path: &Path,
    telex: PathBuf,
    once: bool,
    selected: Vec<String>,
    concurrency: u8,
) -> Result<RunSummary> {
    let lock_path = registry_path.with_extension("lock");
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create Watcher lock directory {}", parent.display()))?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("open Watcher scheduler lock {}", lock_path.display()))?;
    lock.try_lock_exclusive().map_err(|error| {
        anyhow!(
            "another Watcher runtime owns {}: {error}",
            registry_path.display()
        )
    })?;

    let mut registry = Registry::open(registry_path)?;
    let selected: BTreeSet<String> = selected.into_iter().collect();
    for id in &selected {
        require_watch(&registry, id)?;
    }
    let runtime_session_id = fresh_runtime_session_id();
    let watcher_pid = std::process::id();
    // Reconcile the wreckage of any runtime that died abruptly before we record this one, so a
    // fresh runtime never inherits a `running` session row or an unfinished attempt.
    let reconciled = registry.reconcile_interrupted_runtimes()?;
    if !reconciled.is_empty() {
        lifecycle_log(
            "startup-reconciled",
            &runtime_session_id,
            watcher_pid,
            json!({
                "interruptedRuntimes": reconciled.runtime_session_ids,
                "failedAttempts": reconciled.attempt_ids,
                "delayedWatches": reconciled.watch_ids,
            }),
        );
    }
    let runtime_record_id = registry.runtime_started(&runtime_session_id, watcher_pid)?;
    lifecycle_log(
        "runtime-started",
        &runtime_session_id,
        watcher_pid,
        json!({"globalConcurrencyCap": concurrency, "once": once}),
    );

    let adapter = CliTelexAdapter::new(telex);
    let lifecycle = Arc::new(Mutex::new(LifecycleCoordinator::new(
        adapter,
        runtime_session_id.clone(),
        watcher_pid,
    )));
    let shutdown = lifecycle.lock().await.shutdown_signal();
    let _shutdown_listener =
        start_shutdown_listener(shutdown.clone(), runtime_session_id.clone(), watcher_pid);

    let mut desired = sender_set(&registry)?;
    if let Err(error) = reconcile_with_backoff(
        &lifecycle,
        &shutdown,
        &desired,
        &runtime_session_id,
        watcher_pid,
    )
    .await
    {
        lifecycle_log(
            "runtime-not-ready",
            &runtime_session_id,
            watcher_pid,
            json!({"error": error.to_string()}),
        );
        let _ = finish_runtime(
            &mut registry,
            &lifecycle,
            &shutdown,
            RuntimeRecord {
                id: runtime_record_id,
                session_id: &runtime_session_id,
                watcher_pid,
            },
            0,
            "degraded-not-ready",
        )
        .await?;
        return Err(error);
    }
    if !once {
        let jittered = registry.apply_restart_jitter(&selected, now_ms())?;
        if !jittered.is_empty() {
            lifecycle_log(
                "startup-jitter",
                &runtime_session_id,
                watcher_pid,
                json!({
                    "spreadWatches": jittered
                        .iter()
                        .map(|(id, delay_ms)| json!({"watchId": id, "delayMs": delay_ms}))
                        .collect::<Vec<_>>(),
                }),
            );
        }
    }
    // Run the healthy periodic sender reconciliation independently of the scheduler/detector-await
    // loop so it keeps senders attached even while a long detector runs. It shares the lifecycle
    // mutex and shutdown signal, re-reads configured senders each cycle, and exits on shutdown.
    let _periodic_reconcile = if once {
        None
    } else {
        let path = registry_path.to_path_buf();
        Some(AbortOnDrop(tokio::spawn(run_periodic_reconcile(
            Arc::clone(&lifecycle),
            shutdown.clone(),
            Duration::from_secs(5),
            runtime_session_id.clone(),
            watcher_pid,
            move || Registry::open(&path).and_then(|registry| sender_set(&registry)),
        ))))
    };
    let mut last_revision = registry.revision()?;
    let mut runs = 0usize;
    let mut ready = true;

    loop {
        if shutdown.is_requested() {
            break;
        }
        let revision = registry.revision()?;
        if !ready || revision != last_revision {
            desired = sender_set(&registry)?;
            lifecycle_log(
                "registry-revision-reconcile",
                &runtime_session_id,
                watcher_pid,
                json!({"revision": revision, "desiredSenders": desired}),
            );
            if let Err(error) = reconcile_with_backoff(
                &lifecycle,
                &shutdown,
                &desired,
                &runtime_session_id,
                watcher_pid,
            )
            .await
            {
                lifecycle_log(
                    "reconcile-failed",
                    &runtime_session_id,
                    watcher_pid,
                    json!({"error": error.to_string(), "retryAfterSeconds": 30}),
                );
                if once {
                    return finish_runtime(
                        &mut registry,
                        &lifecycle,
                        &shutdown,
                        RuntimeRecord {
                            id: runtime_record_id,
                            session_id: &runtime_session_id,
                            watcher_pid,
                        },
                        runs,
                        "degraded-not-ready",
                    )
                    .await;
                }
                ready = false;
                continue;
            }
            ready = true;
            last_revision = revision;
        }

        let due = if once && !selected.is_empty() {
            registry
                .list()?
                .into_iter()
                .filter(|watch| {
                    watch.status == WatchStatus::Active.as_str() && selected.contains(&watch.id)
                })
                .collect()
        } else {
            registry.due(&selected, now_ms())?
        };
        if !shutdown.is_requested() {
            let attempt_registry_path = registry_path.to_path_buf();
            let attempt_lifecycle = Arc::clone(&lifecycle);
            let attempt_shutdown = shutdown.clone();
            let attempt_desired = desired.clone();
            let attempt_runtime_session_id = runtime_session_id.clone();
            let outcomes = execute_bounded_watch_tasks(
                due.into_iter()
                    .map(|watch| (watch.id.clone(), watch))
                    .collect(),
                concurrency as usize,
                move |watch| {
                    let registry_path = attempt_registry_path.clone();
                    let lifecycle = Arc::clone(&attempt_lifecycle);
                    let shutdown = attempt_shutdown.clone();
                    let desired = attempt_desired.clone();
                    let runtime_session_id = attempt_runtime_session_id.clone();
                    async move {
                        if shutdown.is_requested() {
                            return Ok(());
                        }
                        let mut attempt_registry = Registry::open(&registry_path)?;
                        run_attempt(
                            &mut attempt_registry,
                            &lifecycle,
                            &shutdown,
                            &desired,
                            &runtime_session_id,
                            watcher_pid,
                            watch,
                        )
                        .await
                    }
                },
            )
            .await;
            runs = runs.saturating_add(outcomes.len());
            for (watch_id, outcome) in outcomes {
                if let Err(error) = outcome {
                    lifecycle_log(
                        "attempt-runtime-error",
                        &runtime_session_id,
                        watcher_pid,
                        json!({"watchId": watch_id, "error": error.to_string()}),
                    );
                }
            }
        }

        if once || shutdown.is_requested() {
            break;
        }

        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    finish_runtime(
        &mut registry,
        &lifecycle,
        &shutdown,
        RuntimeRecord {
            id: runtime_record_id,
            session_id: &runtime_session_id,
            watcher_pid,
        },
        runs,
        "stopped",
    )
    .await
}

async fn execute_bounded_watch_tasks<T, F, Fut>(
    work: Vec<(String, T)>,
    concurrency: usize,
    execute: F,
) -> Vec<(String, Result<()>)>
where
    T: Send + 'static,
    F: Fn(T) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    let mut pending: VecDeque<_> = work.into();
    let total = pending.len();
    let mut in_flight = BTreeSet::new();
    let execute = Arc::new(execute);
    let mut tasks = JoinSet::new();
    let mut outcomes = Vec::with_capacity(total);

    while outcomes.len() < total {
        let mut started = true;
        while tasks.len() < concurrency && started {
            started = false;
            for _ in 0..pending.len() {
                let Some((watch_id, payload)) = pending.pop_front() else {
                    break;
                };
                if !in_flight.insert(watch_id.clone()) {
                    pending.push_back((watch_id, payload));
                    continue;
                }
                let execute = Arc::clone(&execute);
                tasks.spawn(async move {
                    let outcome = execute(payload).await;
                    (watch_id, outcome)
                });
                started = true;
                break;
            }
        }

        match tasks.join_next().await {
            Some(Ok((watch_id, outcome))) => {
                in_flight.remove(&watch_id);
                outcomes.push((watch_id, outcome));
            }
            Some(Err(error)) => outcomes.push((
                "<scheduler-task>".to_string(),
                Err(anyhow!("bounded Watcher task failed: {error}")),
            )),
            None => break,
        }
    }
    outcomes
}

/// Independent periodic sender reconciliation loop. Runs off the scheduler/detector-await path so a
/// long-running detector cannot stall healthy reconciliation. Shares the lifecycle mutex and
/// shutdown signal, re-reads the desired sender set each cycle, and retains the bounded ~30s retry
/// behavior of `reconcile_with_backoff`.
async fn run_periodic_reconcile<A, D>(
    lifecycle: SharedLifecycle<A>,
    shutdown: ShutdownSignal,
    interval: Duration,
    runtime_session_id: String,
    watcher_pid: u32,
    desired_provider: D,
) where
    A: adapter::TelexAdapter,
    D: Fn() -> Result<BTreeSet<String>> + Send + 'static,
{
    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = shutdown.cancelled() => break,
        }
        if shutdown.is_requested() {
            break;
        }
        let desired = match desired_provider() {
            Ok(desired) => desired,
            Err(error) => {
                lifecycle_log(
                    "periodic-reconcile-skip",
                    &runtime_session_id,
                    watcher_pid,
                    json!({"error": error.to_string()}),
                );
                continue;
            }
        };
        lifecycle_log(
            "periodic-reconcile",
            &runtime_session_id,
            watcher_pid,
            json!({"desiredSenders": desired}),
        );
        if let Err(error) = reconcile_with_backoff(
            &lifecycle,
            &shutdown,
            &desired,
            &runtime_session_id,
            watcher_pid,
        )
        .await
        {
            lifecycle_log(
                "reconcile-failed",
                &runtime_session_id,
                watcher_pid,
                json!({"error": error.to_string(), "retryAfterSeconds": 30}),
            );
        }
    }
}

async fn reconcile_with_backoff<A: adapter::TelexAdapter>(
    lifecycle: &SharedLifecycle<A>,
    shutdown: &ShutdownSignal,
    desired: &BTreeSet<String>,
    runtime_session_id: &str,
    watcher_pid: u32,
) -> Result<()> {
    let retry_seconds = [1u64, 2, 4, 8, 15];
    let mut last_error = None;
    for attempt in 0..=retry_seconds.len() {
        if shutdown.is_requested() {
            bail!("runtime shutdown began before sender reconciliation");
        }
        let (result, attached, verified) = {
            let mut lifecycle = lifecycle.lock().await;
            let result = lifecycle.reconcile(desired).await;
            (
                result,
                lifecycle.attached().clone(),
                lifecycle.verified().clone(),
            )
        };
        match result {
            Ok(()) => {
                lifecycle_log(
                    "reconcile-ready",
                    runtime_session_id,
                    watcher_pid,
                    json!({
                        "generation": attempt + 1,
                        "desiredSenders": desired,
                        "attachedSenders": attached,
                        "verifiedSenders": verified,
                    }),
                );
                return Ok(());
            }
            Err(error) => {
                last_error = Some(format!("{error:#}"));
                if shutdown.is_requested() {
                    break;
                }
                if let Some(delay) = retry_seconds.get(attempt) {
                    lifecycle_log(
                        "reconcile-retry",
                        runtime_session_id,
                        watcher_pid,
                        json!({
                            "retry": attempt + 1,
                            "error": last_error,
                            "delaySeconds": delay,
                            "attachedSenders": attached,
                            "verifiedSenders": verified,
                        }),
                    );
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(*delay)) => {}
                        _ = shutdown.cancelled() => break,
                    }
                }
            }
        }
    }
    bail!(
        "Watcher is degraded-not-ready after bounded sender attachment retries: {}",
        last_error.unwrap_or_else(|| "unknown failure".to_string())
    )
}

async fn finish_runtime<A: adapter::TelexAdapter>(
    registry: &mut Registry,
    lifecycle: &SharedLifecycle<A>,
    shutdown: &ShutdownSignal,
    runtime: RuntimeRecord<'_>,
    runs: usize,
    status: &str,
) -> Result<RunSummary> {
    shutdown.request();
    let detached = lifecycle
        .lock()
        .await
        .shutdown()
        .await
        .into_iter()
        .map(|(sender, outcome)| DetachOutcome { sender, outcome })
        .collect::<Vec<_>>();
    let final_status = if detached.iter().all(|outcome| outcome.outcome == "detached") {
        status
    } else {
        "degraded-shutdown"
    };
    let detail = json!({
        "runtimeSessionId": runtime.session_id,
        "detachOutcomes": detached,
        "runs": runs,
    });
    registry.runtime_finished(runtime.id, final_status, &detail)?;
    lifecycle_log(
        "runtime-stopped",
        runtime.session_id,
        runtime.watcher_pid,
        detail,
    );
    Ok(RunSummary {
        runtime_session_id: runtime.session_id.to_owned(),
        watcher_pid: runtime.watcher_pid,
        runs,
        detached,
    })
}

fn sender_set(registry: &Registry) -> Result<BTreeSet<String>> {
    Ok(registry.configured_senders()?.into_iter().collect())
}

async fn run_attempt<A: adapter::TelexAdapter>(
    registry: &mut Registry,
    lifecycle: &SharedLifecycle<A>,
    shutdown: &ShutdownSignal,
    desired: &BTreeSet<String>,
    runtime_session_id: &str,
    watcher_pid: u32,
    watch: Watch,
) -> Result<()> {
    if shutdown.is_requested() {
        return Ok(());
    }
    let attempt_id = Uuid::new_v4().to_string();
    let before_digest = match script_digest(&watch.script_path) {
        Ok(digest) => digest,
        Err(error) => {
            let prior_hash = registry.begin_attempt(&attempt_id, &watch, None)?;
            registry.record_failure(
                &watch,
                &attempt_id,
                "script-read-failed",
                &error.to_string(),
                None,
                None,
            )?;
            lifecycle_log(
                "attempt-failed",
                runtime_session_id,
                watcher_pid,
                json!({"watchId": watch.id, "attemptId": attempt_id, "priorStateHash": prior_hash, "error": error.to_string()}),
            );
            return Ok(());
        }
    };
    let prior_state_hash = registry.begin_attempt(&attempt_id, &watch, Some(&before_digest))?;
    if watch.script_mode == ScriptMode::Pinned
        && watch.script_digest.as_deref() != Some(before_digest.as_str())
    {
        let diagnostic = "pinned script digest does not match the registered digest";
        registry.record_failure(
            &watch,
            &attempt_id,
            "pinned-digest-mismatch",
            diagnostic,
            None,
            Some(&before_digest),
        )?;
        return Ok(());
    }

    let request = DetectorRequest {
        schema_version: protocol::SCHEMA_VERSION,
        attempt: protocol::AttemptRequest {
            id: attempt_id.clone(),
            now: chrono::Utc::now().to_rfc3339(),
        },
        watch: WatchRequest {
            id: watch.id.clone(),
            parameters: watch.parameters.clone(),
        },
        script: ScriptRequest {
            mode: watch.script_mode.as_str().to_string(),
            sha256: before_digest.clone(),
        },
        state: watch.state.clone(),
    };
    let input = serde_json::to_vec(&request)?;
    let output = match execute_detector(&watch, &input, shutdown).await {
        Ok(output) => output,
        Err(error) => {
            registry.record_failure(
                &watch,
                &attempt_id,
                "execution-failed",
                &error.to_string(),
                None,
                Some(&before_digest),
            )?;
            lifecycle_log(
                "attempt-failed",
                runtime_session_id,
                watcher_pid,
                json!({"watchId": watch.id, "attemptId": attempt_id, "error": error.to_string()}),
            );
            return Ok(());
        }
    };
    let result_value: Value = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(error) => {
            let diagnostic = diagnostic_with_stderr(
                &format!("invalid detector JSON: {error}"),
                &output.stderr,
                &watch,
            );
            registry.record_failure(
                &watch,
                &attempt_id,
                "malformed-output",
                &diagnostic,
                None,
                Some(&before_digest),
            )?;
            return Ok(());
        }
    };
    let result = match parse_result(&output.stdout) {
        Ok(result) => result,
        Err(error) => {
            let diagnostic = diagnostic_with_stderr(&error.to_string(), &output.stderr, &watch);
            registry.record_failure(
                &watch,
                &attempt_id,
                "invalid-result",
                &diagnostic,
                Some(&result_value),
                Some(&before_digest),
            )?;
            return Ok(());
        }
    };
    let after_digest = match script_digest(&watch.script_path) {
        Ok(digest) => digest,
        Err(error) => {
            registry.record_failure(
                &watch,
                &attempt_id,
                "script-read-failed",
                &error.to_string(),
                Some(&result_value),
                Some(&before_digest),
            )?;
            return Ok(());
        }
    };
    if after_digest != before_digest {
        registry.record_failure(
            &watch,
            &attempt_id,
            "script-drift",
            "script changed while detector was running",
            Some(&result_value),
            Some(&before_digest),
        )?;
        return Ok(());
    }
    apply_result(
        registry,
        lifecycle,
        shutdown,
        desired,
        runtime_session_id,
        watcher_pid,
        &watch,
        &attempt_id,
        &prior_state_hash,
        &before_digest,
        &output.stderr,
        result,
        result_value,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn apply_result<A: adapter::TelexAdapter>(
    registry: &mut Registry,
    lifecycle: &SharedLifecycle<A>,
    shutdown: &ShutdownSignal,
    desired: &BTreeSet<String>,
    runtime_session_id: &str,
    watcher_pid: u32,
    watch: &Watch,
    attempt_id: &str,
    prior_state_hash: &str,
    script_digest: &str,
    stderr: &[u8],
    result: ValidatedResult,
    result_value: Value,
) -> Result<()> {
    if shutdown.is_requested() {
        registry.record_failure(
            watch,
            attempt_id,
            "shutdown-interrupted",
            "runtime shutdown began before detector result was accepted",
            Some(&result_value),
            Some(script_digest),
        )?;
        return Ok(());
    }
    match result.outcome {
        Outcome::Degraded => {
            let diagnostic = diagnostic_with_stderr("detector reported degraded", stderr, watch);
            registry.record_failure(
                watch,
                attempt_id,
                "degraded",
                &diagnostic,
                Some(&result_value),
                Some(script_digest),
            )?;
        }
        Outcome::Idle => {
            registry.commit_idle(
                watch,
                attempt_id,
                result.next_state.unwrap_or_else(|| watch.state.clone()),
                false,
                &result_value,
            )?;
        }
        Outcome::Terminal if result.event.is_none() => {
            registry.commit_idle(
                watch,
                attempt_id,
                result.next_state.unwrap_or_else(|| watch.state.clone()),
                true,
                &result_value,
            )?;
        }
        Outcome::Event | Outcome::Terminal => {
            let event = result
                .event
                .ok_or_else(|| anyhow!("validated event result unexpectedly lacks an event"))?;
            let envelope = normalized_envelope(&watch.id, &watch.sender, &watch.target, &event)?;
            let envelope_hash = hash_value(&envelope)?;
            if let Some(committed) = registry.event(&watch.id, &event.id)? {
                let (outcome, diagnostic) = if committed.envelope_hash == envelope_hash {
                    (
                        "stale-duplicate",
                        "event ID was already committed with the same envelope hash",
                    )
                } else {
                    (
                        "duplicate-event-conflict",
                        "event ID was already committed with different event evidence",
                    )
                };
                registry.finish_noop(
                    watch,
                    attempt_id,
                    outcome,
                    Some(&result_value),
                    Some(&event.id),
                    Some(&envelope_hash),
                    Some(diagnostic),
                )?;
                lifecycle_log(
                    outcome,
                    runtime_session_id,
                    watcher_pid,
                    json!({
                        "watchId": watch.id,
                        "attemptId": attempt_id,
                        "eventId": event.id,
                        "committedEnvelopeHash": committed.envelope_hash,
                        "proposedEnvelopeHash": envelope_hash,
                    }),
                );
                return Ok(());
            }
            let metadata = match send_metadata(
                &watch.id,
                attempt_id,
                watch.script_mode.as_str(),
                script_digest,
                &event,
            ) {
                Ok(metadata) => metadata,
                Err(error) => {
                    registry.record_failure(
                        watch,
                        attempt_id,
                        "normalization-failed",
                        &error.to_string(),
                        Some(&result_value),
                        Some(script_digest),
                    )?;
                    return Ok(());
                }
            };
            let request = SendRequest {
                sender: watch.sender.clone(),
                target: watch.target.clone(),
                kind: event.kind.clone(),
                attention: watch.attention.clone(),
                requires_disposition: watch.requires_disposition,
                subject: event.subject.clone(),
                body: event.body.clone(),
                metadata,
            };
            let receipt = match lifecycle.lock().await.send(desired, &request).await {
                Ok(receipt)
                    if matches!(receipt.receipt.as_str(), "delivered" | "queued-unoccupied") =>
                {
                    receipt
                }
                Ok(receipt) => {
                    registry.record_failure(
                        watch,
                        attempt_id,
                        "unknown-receipt",
                        &format!("Telex returned unsupported receipt {:?}", receipt.receipt),
                        Some(&result_value),
                        Some(script_digest),
                    )?;
                    return Ok(());
                }
                Err(error) => {
                    registry.record_failure(
                        watch,
                        attempt_id,
                        "send-failed",
                        &error.to_string(),
                        Some(&result_value),
                        Some(script_digest),
                    )?;
                    return Ok(());
                }
            };
            let receipt_json = serde_json::to_value(&receipt)?;
            registry.commit_event(
                watch,
                attempt_id,
                &event.id,
                prior_state_hash,
                result.next_state.unwrap_or_else(|| watch.state.clone()),
                &envelope_hash,
                script_digest,
                receipt.id,
                &receipt_json,
                result.outcome == Outcome::Terminal,
                &result_value,
            )?;
        }
    }
    Ok(())
}

async fn execute_detector(
    watch: &Watch,
    input: &[u8],
    shutdown: &ShutdownSignal,
) -> Result<ProcessOutput> {
    if shutdown.is_requested() {
        bail!("runtime shutdown began before detector execution");
    }
    let program = watch
        .command
        .first()
        .ok_or_else(|| anyhow!("watch command is empty"))?;
    let mut command = Command::new(program);
    command.args(&watch.command[1..]);
    command.current_dir(&watch.working_directory);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.env_clear();
    inherit_launch_baseline(&mut command);
    let sensitive_values =
        inherit_allowlisted_environment(&mut command, &watch.environment_allowlist);
    configure_process_group(&mut command)?;
    #[cfg(windows)]
    let process_group = create_windows_job()?;

    let mut child = command
        .spawn()
        .with_context(|| format!("start detector for watch {:?}", watch.id))?;
    #[cfg(windows)]
    if let Err(error) = assign_windows_job(&process_group, &child) {
        let _ = child.kill().await;
        let _ = child.wait().await;
        return Err(error).context("assign detector to Windows Job Object");
    }
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("detector stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("detector stderr was not piped"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("detector stdin was not piped"))?;
    let request = input.to_vec();
    let writer = tokio::spawn(async move {
        stdin.write_all(&request).await?;
        stdin.shutdown().await
    });
    // Read both streams concurrently and short-circuit on the first cap breach with `try_join!`.
    // Only wait for the child once both pipes drain cleanly; a cap breach or timeout terminates
    // the whole process group immediately instead of deadlocking on a child blocked writing into
    // a full pipe.
    let completed = {
        let read_and_wait = async {
            let (stdout, stderr) = tokio::try_join!(
                read_capped(stdout, MAX_STDOUT_BYTES),
                read_capped(stderr, MAX_STDERR_BYTES),
            )?;
            let status = child.wait().await.context("wait for detector")?;
            Ok::<_, anyhow::Error>((stdout, stderr, status))
        };
        tokio::pin!(read_and_wait);
        tokio::select! {
            result = tokio::time::timeout(Duration::from_secs(watch.timeout_seconds), &mut read_and_wait) => Some(result),
            _ = shutdown.cancelled() => None,
        }
    };
    let (stdout, stderr, status) = match completed {
        Some(Ok(Ok(value))) => value,
        Some(Ok(Err(error))) => {
            #[cfg(windows)]
            terminate_process_group(&mut child, &process_group).await;
            #[cfg(not(windows))]
            terminate_process_group(&mut child).await;
            writer.abort();
            return Err(error);
        }
        Some(Err(_)) => {
            #[cfg(windows)]
            terminate_process_group(&mut child, &process_group).await;
            #[cfg(not(windows))]
            terminate_process_group(&mut child).await;
            writer.abort();
            bail!("detector timed out after {} seconds", watch.timeout_seconds);
        }
        None => {
            #[cfg(windows)]
            terminate_process_group(&mut child, &process_group).await;
            #[cfg(not(windows))]
            terminate_process_group(&mut child).await;
            writer.abort();
            bail!("detector interrupted by runtime shutdown");
        }
    };
    match writer.await {
        Ok(result) => result.context("write detector request")?,
        Err(join_error) => bail!("detector stdin writer task failed: {join_error}"),
    }
    if !status.success() {
        bail!(
            "detector exited with {status}: {}",
            redact_text(&String::from_utf8_lossy(&stderr), &sensitive_values)
        );
    }
    Ok(ProcessOutput {
        stdout,
        stderr: redact_bytes(stderr, &sensitive_values),
    })
}

async fn read_capped<R: AsyncRead + Unpin>(mut reader: R, cap: usize) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(cap.min(8192));
    let mut buffer = [0u8; 8192];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(count) > cap {
            bail!("process output exceeded {cap} bytes");
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

fn inherit_launch_baseline(command: &mut Command) {
    // Restore a minimal but functional platform baseline so trusted detectors and the tools they
    // shell out to (git, gh, az, node, python) start normally from a cleared environment. This
    // intentionally excludes credentials, which flow only through the per-watch allowlist.
    const BASELINE_KEYS: &[&str] = &[
        // Executable search and core system roots.
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "SystemDrive",
        "WINDIR",
        "ComSpec",
        "OS",
        "NUMBER_OF_PROCESSORS",
        "PROCESSOR_ARCHITECTURE",
        "PROCESSOR_ARCHITEW6432",
        "PROCESSOR_IDENTIFIER",
        // Temporary directories.
        "TMP",
        "TEMP",
        "TMPDIR",
        // Home and per-user configuration roots.
        "HOME",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "APPDATA",
        "LOCALAPPDATA",
        "ProgramData",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "XDG_RUNTIME_DIR",
        // Identity, locale, and shell that many CLIs read for defaults.
        "USER",
        "USERNAME",
        "LOGNAME",
        "USERDOMAIN",
        "LANG",
        "LANGUAGE",
        "LC_ALL",
        "LC_CTYPE",
        "LC_MESSAGES",
        "TZ",
        "SHELL",
        "TERM",
    ];
    for key in BASELINE_KEYS {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
}

fn inherit_allowlisted_environment(command: &mut Command, allowlist: &[String]) -> Vec<String> {
    let mut sensitive_values = Vec::new();
    for name in allowlist {
        if let Some(value) = std::env::var_os(name) {
            if is_sensitive_name(name) {
                sensitive_values.push(value.to_string_lossy().into_owned());
            }
            command.env(name, value);
        }
    }
    sensitive_values
}

fn is_sensitive_name(name: &str) -> bool {
    // Sensitive-name detection is case-insensitive so lowercase or mixed-case allowlist entries
    // (for example `gh_token`) are still redacted from bounded stderr.
    let upper = name.to_ascii_uppercase();
    upper == "GH_TOKEN"
        || upper == "AZURE_DEVOPS_EXT_PAT"
        || ["TOKEN", "PAT", "KEY", "SECRET"]
            .iter()
            .any(|suffix| upper.ends_with(suffix))
}

fn redact_text(text: &str, sensitive_values: &[String]) -> String {
    sensitive_values
        .iter()
        .filter(|value| !value.is_empty())
        .fold(text.to_string(), |redacted, value| {
            redacted.replace(value, "[redacted]")
        })
}

fn redact_bytes(bytes: Vec<u8>, sensitive_values: &[String]) -> Vec<u8> {
    redact_text(&String::from_utf8_lossy(&bytes), sensitive_values).into_bytes()
}

fn diagnostic_with_stderr(prefix: &str, stderr: &[u8], watch: &Watch) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let values = watch
        .environment_allowlist
        .iter()
        .filter(|name| is_sensitive_name(name))
        .filter_map(|name| std::env::var(name).ok())
        .collect::<Vec<_>>();
    let suffix = redact_text(&stderr, &values);
    if suffix.trim().is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}; stderr: {}", suffix.trim())
    }
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) -> Result<()> {
    use std::os::unix::process::CommandExt;
    unsafe {
        command.as_std_mut().pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn configure_process_group(_: &mut Command) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
async fn terminate_process_group(child: &mut Child) {
    if let Some(pid) = child.id() {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

#[cfg(all(not(unix), not(windows)))]
async fn terminate_process_group(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

#[cfg(windows)]
struct WindowsJob(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn create_windows_job() -> Result<WindowsJob> {
    use std::ptr::null;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    let handle = unsafe { CreateJobObjectW(null(), null()) };
    if handle == 0 {
        return Err(std::io::Error::last_os_error()).context("create Windows Job Object");
    }
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let configured = unsafe {
        SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            std::mem::size_of_val(&limits) as u32,
        )
    };
    if configured == 0 {
        let error = std::io::Error::last_os_error();
        unsafe {
            CloseHandle(handle);
        }
        return Err(error).context("configure Windows Job Object");
    }
    Ok(WindowsJob(handle))
}

#[cfg(windows)]
fn assign_windows_job(job: &WindowsJob, child: &Child) -> Result<()> {
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

    let handle = child
        .raw_handle()
        .ok_or_else(|| anyhow!("detector process has no Windows handle"))?;
    if unsafe { AssignProcessToJobObject(job.0, handle as _) } == 0 {
        return Err(std::io::Error::last_os_error())
            .context("assign process to Windows Job Object");
    }
    Ok(())
}

#[cfg(windows)]
async fn terminate_process_group(child: &mut Child, job: &WindowsJob) {
    use windows_sys::Win32::System::JobObjects::TerminateJobObject;

    unsafe {
        TerminateJobObject(job.0, 1);
    }
    let _ = child.wait().await;
}

fn lifecycle_log(event: &str, runtime_session_id: &str, watcher_pid: u32, detail: Value) {
    let record = json!({
        "event": event,
        "at_ms": now_ms(),
        "runtime_session_id": runtime_session_id,
        "watcher_pid": watcher_pid,
        "detail": detail,
    });
    match serde_json::to_string(&record) {
        Ok(line) => eprintln!("{line}"),
        Err(error) => eprintln!(
            "{{\"event\":\"logging-failed\",\"error\":{:?}}}",
            error.to_string()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use registry::SentEvent;
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::Mutex as StdMutex;

    #[test]
    fn sensitive_values_are_redacted() {
        assert_eq!(
            redact_text("token=abc", &["abc".to_string()]),
            "token=[redacted]"
        );
    }

    #[test]
    fn stale_duplicate_compares_only_committed_envelope_hash() {
        let committed = SentEvent {
            watch_id: "watch".into(),
            event_id: "event".into(),
            prior_state_hash: "prior".into(),
            next_state_hash: "committed-next".into(),
            envelope_hash: "same-envelope".into(),
            script_digest: "digest".into(),
            sender: "sender".into(),
            target: "target".into(),
            message_id: 1,
            receipt_json: json!({}),
            attempt_id: "first".into(),
            accepted_at_ms: 0,
        };
        assert_eq!(committed.envelope_hash, "same-envelope");
        assert_ne!(committed.next_state_hash, "newly-proposed-next");
    }

    #[test]
    fn watcher_is_experimental_binary_only() {
        let manifest = fs::read_to_string("Cargo.toml").unwrap();
        assert!(manifest.contains("Experimental trusted-local detector runner"));
    }

    #[test]
    fn fresh_process_start_runtime_ids_are_distinct_uuids() {
        let first = fresh_runtime_session_id();
        let second = fresh_runtime_session_id();

        assert_ne!(first, second);
        Uuid::parse_str(&first).unwrap();
        Uuid::parse_str(&second).unwrap();
    }

    #[tokio::test]
    async fn bounded_watch_tasks_parallelize_distinct_watches_without_overlap() {
        #[derive(Default)]
        struct Probe {
            active: usize,
            max_active: usize,
            active_by_watch: BTreeMap<String, usize>,
            same_watch_overlap: bool,
        }

        let probe = Arc::new(StdMutex::new(Probe::default()));
        let outcomes = execute_bounded_watch_tasks(
            vec![
                ("watch-a".to_string(), ("watch-a".to_string(), 40u64)),
                ("watch-b".to_string(), ("watch-b".to_string(), 40u64)),
                ("watch-a".to_string(), ("watch-a".to_string(), 40u64)),
                ("watch-c".to_string(), ("watch-c".to_string(), 40u64)),
            ],
            2,
            {
                let probe = Arc::clone(&probe);
                move |(watch_id, delay_ms)| {
                    let probe = Arc::clone(&probe);
                    async move {
                        {
                            let mut state = probe.lock().unwrap();
                            state.active += 1;
                            state.max_active = state.max_active.max(state.active);
                            let active_for_watch =
                                state.active_by_watch.entry(watch_id.clone()).or_default();
                            *active_for_watch += 1;
                            state.same_watch_overlap |= *active_for_watch > 1;
                        }
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        let mut state = probe.lock().unwrap();
                        state.active -= 1;
                        *state.active_by_watch.get_mut(&watch_id).unwrap() -= 1;
                        Ok(())
                    }
                }
            },
        )
        .await;

        assert!(outcomes.iter().all(|(_, outcome)| outcome.is_ok()));
        let state = probe.lock().unwrap();
        assert!(
            state.max_active > 1,
            "distinct watches should execute concurrently"
        );
        assert!(state.max_active <= 2, "configured cap must bound execution");
        assert!(
            !state.same_watch_overlap,
            "a watch must never have two in-flight attempts"
        );
    }

    #[test]
    fn sensitive_name_detection_is_case_insensitive() {
        for sensitive in [
            "GH_TOKEN",
            "gh_token",
            "Azure_DevOps_Ext_Pat",
            "MY_SECRET",
            "my_secret",
            "service_api_key",
            "DEPLOY_PAT",
        ] {
            assert!(
                is_sensitive_name(sensitive),
                "{sensitive} should be sensitive"
            );
        }
        for benign in ["PATH", "HOME", "LANG", "WATCH_INTERVAL"] {
            assert!(
                !is_sensitive_name(benign),
                "{benign} should not be sensitive"
            );
        }
    }

    fn sleeper_watch() -> Watch {
        #[cfg(windows)]
        let command = vec![
            "cmd".to_string(),
            "/c".to_string(),
            "ping".to_string(),
            "-n".to_string(),
            "30".to_string(),
            "127.0.0.1".to_string(),
        ];
        #[cfg(not(windows))]
        let command = vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "sleep 30".to_string(),
        ];
        Watch {
            id: "sleeper".into(),
            command,
            script_path: std::path::PathBuf::from("."),
            working_directory: std::path::PathBuf::from("."),
            script_mode: ScriptMode::FollowPath,
            script_digest: None,
            sender: "service:watcher".into(),
            target: "target".into(),
            interval_seconds: 60,
            // A large timeout guarantees only the shutdown signal — not the detector timeout —
            // can end this attempt, so the assertion below proves shutdown cancellation works.
            timeout_seconds: 3600,
            attention: "background".into(),
            requires_disposition: false,
            environment_allowlist: Vec::new(),
            parameters: json!({}),
            state: json!({}),
            status: "active".into(),
            next_due_ms: 0,
            failure_count: 0,
            last_diagnostic: None,
            updated_at_ms: 0,
        }
    }

    #[tokio::test]
    async fn shutdown_cancels_a_running_detector_without_waiting_for_timeout() {
        let watch = sleeper_watch();
        let shutdown = ShutdownSignal::default();
        let trigger = shutdown.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            trigger.request();
        });
        let start = std::time::Instant::now();
        let result = execute_detector(&watch, b"{}\n", &shutdown).await;
        let elapsed = start.elapsed();
        let error = result
            .expect_err("cancelled detector must fail")
            .to_string();
        assert!(
            error.contains("interrupted by runtime shutdown"),
            "unexpected error: {error}"
        );
        assert!(
            elapsed < Duration::from_secs(15),
            "shutdown must not wait for the detector timeout, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn periodic_reconcile_runs_while_a_detector_is_blocked() {
        use adapter::{SendReceipt, SendResult, TelexAdapter};
        use async_trait::async_trait;

        #[derive(Clone, Default)]
        struct CountingAdapter {
            attaches: Arc<StdMutex<usize>>,
        }

        #[async_trait]
        impl TelexAdapter for CountingAdapter {
            async fn attach(&self, _: &str, _: &str, _: u32) -> Result<()> {
                *self.attaches.lock().unwrap() += 1;
                Ok(())
            }
            async fn verify_attached(&self, _: &str, _: &str, _: u32) -> Result<()> {
                Ok(())
            }
            async fn strict_send(&self, _: &str, _: &SendRequest) -> Result<SendResult> {
                Ok(SendResult::Accepted {
                    receipt: SendReceipt {
                        receipt: "ok".into(),
                        id: 1,
                        thread_id: 1,
                        to: "target".into(),
                        from: None,
                    },
                })
            }
            async fn detach(&self, _: &str, _: &str) -> Result<()> {
                Ok(())
            }
        }

        let adapter = CountingAdapter::default();
        let counter = Arc::clone(&adapter.attaches);
        let lifecycle = Arc::new(Mutex::new(LifecycleCoordinator::new(
            adapter,
            "runtime".into(),
            7,
        )));
        let shutdown = lifecycle.lock().await.shutdown_signal();
        let desired = BTreeSet::from(["service:watcher".to_string()]);
        let provider = move || Ok(desired.clone());
        let handle = tokio::spawn(run_periodic_reconcile(
            Arc::clone(&lifecycle),
            shutdown.clone(),
            Duration::from_millis(50),
            "runtime".into(),
            7,
            provider,
        ));

        // While a long detector would block the scheduler loop, the independent reconcile keeps
        // running: several attach cycles must complete during this window.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            *counter.lock().unwrap() >= 2,
            "periodic reconcile must run while a detector is blocked, saw {} attaches",
            *counter.lock().unwrap()
        );

        shutdown.request();
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("periodic reconcile must exit promptly on shutdown")
            .expect("periodic reconcile task must not panic");
    }

    #[tokio::test(start_paused = true)]
    async fn reconcile_backoff_is_bounded_and_returns_err_not_a_busy_loop() {
        use adapter::{SendResult, TelexAdapter};
        use async_trait::async_trait;

        #[derive(Clone, Default)]
        struct AlwaysFailAdapter;

        #[async_trait]
        impl TelexAdapter for AlwaysFailAdapter {
            async fn attach(&self, sender: &str, _: &str, _: u32) -> Result<()> {
                bail!("forced attach failure for {sender}")
            }
            async fn verify_attached(&self, _: &str, _: &str, _: u32) -> Result<()> {
                Ok(())
            }
            async fn strict_send(&self, _: &str, _: &SendRequest) -> Result<SendResult> {
                unreachable!("reconcile never sends")
            }
            async fn detach(&self, _: &str, _: &str) -> Result<()> {
                Ok(())
            }
        }

        let lifecycle = Arc::new(Mutex::new(LifecycleCoordinator::new(
            AlwaysFailAdapter,
            "runtime".into(),
            3,
        )));
        let shutdown = lifecycle.lock().await.shutdown_signal();
        let desired = BTreeSet::from(["service:watcher".to_string()]);

        let start = tokio::time::Instant::now();
        let result = reconcile_with_backoff(&lifecycle, &shutdown, &desired, "runtime", 3).await;
        let elapsed = start.elapsed();

        assert!(
            result.is_err(),
            "bounded reconcile retries must ultimately return an error"
        );
        // The five backoff sleeps sum to 1+2+4+8+15 = 30s. Under the paused test clock this is the
        // exact virtual elapsed time, proving the loop sleeps (not a 250ms busy loop) and is bounded.
        assert_eq!(
            elapsed,
            Duration::from_secs(30),
            "reconcile backoff must be a bounded ~30s, got {elapsed:?}"
        );
    }
}
