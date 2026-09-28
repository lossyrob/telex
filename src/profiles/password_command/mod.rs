use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::Process;
#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix::Process;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Failure {
    pub(super) stage: &'static str,
    pub(super) os_code: Option<i32>,
}

impl Failure {
    pub(super) fn io(stage: &'static str, error: std::io::Error) -> Self {
        Self {
            stage,
            os_code: error.raw_os_error(),
        }
    }

    pub(super) fn at(stage: &'static str) -> Self {
        Self {
            stage,
            os_code: None,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "credential command {}", self.stage)?;
        if let Some(code) = self.os_code {
            write!(f, " (code {code})")?;
        }
        Ok(())
    }
}

impl std::error::Error for Failure {}

const CAPACITY: usize = 2;
const CLEANUP_BUDGET: Duration = Duration::from_secs(3);
const CHECK_INTERVAL: Duration = Duration::from_millis(5);

struct WorkerResult {
    scope: Process,
    cleaned: bool,
    outcome: Result<String, Failure>,
}

enum Publication {
    Pending,
    Cancelled,
    Completed(Option<Result<String, Failure>>),
    Taken,
}

struct Invocation {
    id: u64,
    source: Arc<str>,
    cancelled: AtomicBool,
    publication: Mutex<Publication>,
    cleanup_deadline: Mutex<Option<Instant>>,
    worker: Mutex<Option<JoinHandle<WorkerResult>>>,
    joining: AtomicBool,
    joined: AtomicBool,
    held_scope: Mutex<Option<Process>>,
    failure: Mutex<Option<Failure>>,
    #[cfg(test)]
    publication_gate: Mutex<Option<Arc<PublicationGate>>>,
}

#[cfg(test)]
struct PublicationGate {
    after_close: std::sync::Barrier,
    release: std::sync::Barrier,
}

impl Invocation {
    fn cancel(&self, deadline: Instant) {
        let mut publication = self.publication.lock().unwrap();
        if matches!(*publication, Publication::Taken) {
            return;
        }
        *publication = Publication::Cancelled;
        self.cancelled.store(true, Ordering::Release);
        self.limit_cleanup(deadline);
    }

    fn limit_cleanup(&self, limit: Instant) -> Instant {
        let mut deadline = self.cleanup_deadline.lock().unwrap();
        let selected = deadline.map_or(limit, |current| current.min(limit));
        *deadline = Some(selected);
        selected
    }

    fn cleanup_limit(&self) -> Instant {
        self.limit_cleanup(Instant::now() + CLEANUP_BUDGET)
    }

    fn mark_failure(&self, failure: &Failure) -> bool {
        let publication = self.publication.lock().unwrap();
        if matches!(*publication, Publication::Taken) {
            return false;
        }
        let mut current = self.failure.lock().unwrap();
        if current.is_some() {
            false
        } else {
            *current = Some(failure.clone());
            true
        }
    }

    fn report_failure(&self, failure: &Failure) {
        eprintln!(
            "[telex] credential invocation {} FAILED_HELD: {failure}; \
                 owning host {} retains cleanup responsibility",
            self.id,
            std::process::id(),
        );
    }

    fn record_failure(&self, failure: Failure) {
        if self.mark_failure(&failure) {
            self.report_failure(&failure);
        }
    }
}

struct CancelOnDrop {
    invocation: Arc<Invocation>,
    armed: bool,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.invocation.cancel(Instant::now() + CLEANUP_BUDGET);
        }
    }
}

struct RegistryState {
    next_id: u64,
    closed: bool,
    invocations: Vec<Arc<Invocation>>,
}

struct Registry {
    state: Mutex<RegistryState>,
    changed: Arc<Notify>,
}

impl Registry {
    fn new() -> Self {
        Self {
            state: Mutex::new(RegistryState {
                next_id: 1,
                closed: false,
                invocations: Vec::new(),
            }),
            changed: Arc::new(Notify::new()),
        }
    }

    fn collect_finished(&self) {
        let invocations = self.state.lock().unwrap().invocations.clone();
        for invocation in invocations {
            let handle = {
                let mut worker = invocation.worker.lock().unwrap();
                if !worker.as_ref().is_some_and(JoinHandle::is_finished)
                    || invocation.joining.swap(true, Ordering::AcqRel)
                {
                    continue;
                }
                worker.take().unwrap()
            };
            // Only a finished native worker is joined. No registry lock crosses a join.
            let joined = handle.join();
            invocation.joined.store(true, Ordering::Release);
            match joined {
                Ok(result) if result.cleaned => {
                    let outcome = result.outcome;
                    drop(result.scope);
                    #[cfg(test)]
                    if let Some(gate) = invocation.publication_gate.lock().unwrap().clone() {
                        gate.after_close.wait();
                        gate.release.wait();
                    }
                    let mut state = self.state.lock().unwrap();
                    let mut publication = invocation.publication.lock().unwrap();
                    if invocation.failure.lock().unwrap().is_none() {
                        if matches!(*publication, Publication::Pending) {
                            *publication = Publication::Completed(Some(outcome));
                        }
                        state.invocations.retain(|entry| entry.id != invocation.id);
                    }
                }
                Ok(result) => {
                    let failure = result
                        .outcome
                        .err()
                        .unwrap_or_else(|| Failure::at("cleanup receipt missing"));
                    *invocation.held_scope.lock().unwrap() = Some(result.scope);
                    invocation.record_failure(failure);
                }
                Err(_) => {
                    let failure = Failure::at("native owner failed without a cleanup receipt");
                    invocation.record_failure(failure);
                }
            }
            self.changed.notify_waiters();
        }
    }

    fn reserve(&self, source: &str) -> Result<Option<Arc<Invocation>>, Failure> {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return Err(Failure::at("admission closed for owning-host shutdown"));
        }
        if let Some(existing) = state
            .invocations
            .iter()
            .find(|entry| &*entry.source == source)
        {
            if let Some(failure) = existing.failure.lock().unwrap().clone() {
                return Err(failure);
            }
            return Ok(None);
        }
        if state.invocations.len() == CAPACITY {
            if state
                .invocations
                .iter()
                .all(|entry| entry.failure.lock().unwrap().is_some())
            {
                return Err(Failure::at(
                    "admission held by unresolved cleanup obligations",
                ));
            }
            return Ok(None);
        }
        let id = state.next_id;
        state.next_id = id
            .checked_add(1)
            .ok_or_else(|| Failure::at("identity exhausted"))?;
        let invocation = Arc::new(Invocation {
            id,
            source: Arc::from(source),
            cancelled: AtomicBool::new(false),
            publication: Mutex::new(Publication::Pending),
            cleanup_deadline: Mutex::new(None),
            worker: Mutex::new(None),
            joining: AtomicBool::new(false),
            joined: AtomicBool::new(false),
            held_scope: Mutex::new(None),
            failure: Mutex::new(None),
            #[cfg(test)]
            publication_gate: Mutex::new(None),
        });
        state.invocations.push(invocation.clone());
        Ok(Some(invocation))
    }

    async fn execute(&self, command: &str) -> Result<String, Failure> {
        let invocation = loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            self.collect_finished();
            if let Some(invocation) = self.reserve(command)? {
                break invocation;
            }
            tokio::select! {
                _ = notified => {},
                _ = tokio::time::sleep(CHECK_INTERVAL) => {},
            }
        };
        let mut cancellation = CancelOnDrop {
            invocation: invocation.clone(),
            armed: true,
        };
        let owned = invocation.clone();
        let changed = self.changed.clone();
        let worker = thread::Builder::new()
            .name(format!("telex-credential-{}", invocation.id))
            .spawn(move || {
                let result = run_owned(&owned);
                changed.notify_waiters();
                result
            });
        match worker {
            Ok(worker) => *invocation.worker.lock().unwrap() = Some(worker),
            Err(error) => {
                self.state
                    .lock()
                    .unwrap()
                    .invocations
                    .retain(|entry| entry.id != invocation.id);
                self.changed.notify_waiters();
                return Err(Failure::io("native owner creation failed", error));
            }
        }
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            self.collect_finished();
            let failure = invocation.failure.lock().unwrap().clone();
            if let Some(failure) = failure {
                return Err(failure);
            }
            {
                let mut publication = invocation.publication.lock().unwrap();
                if let Publication::Completed(result) = &mut *publication {
                    let result = result.take().unwrap();
                    *publication = Publication::Taken;
                    cancellation.armed = false;
                    return result;
                }
                if matches!(*publication, Publication::Cancelled) {
                    return Err(Failure::at("cancelled"));
                }
            }
            tokio::select! {
                _ = notified => {},
                _ = tokio::time::sleep(CHECK_INTERVAL) => {},
            }
        }
    }

    fn shutdown(&self) -> Result<(), Vec<Obligation>> {
        let deadline = Instant::now() + CLEANUP_BUDGET;
        let invocations = {
            let mut state = self.state.lock().unwrap();
            state.closed = true;
            state.invocations.clone()
        };
        for invocation in invocations {
            invocation.cancel(deadline);
        }
        self.changed.notify_waiters();
        loop {
            self.collect_finished();
            let pending = self.obligations();
            if pending.is_empty() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                let failure = Failure::at("owning-host cleanup budget exhausted");
                let newly_failed = {
                    let state = self.state.lock().unwrap();
                    state
                        .invocations
                        .iter()
                        .filter(|entry| entry.mark_failure(&failure))
                        .cloned()
                        .collect::<Vec<_>>()
                };
                for invocation in newly_failed {
                    invocation.report_failure(&failure);
                }
                return Err(self.obligations());
            }
            if pending
                .iter()
                .all(|entry| entry.failed_held && entry.native_finished)
            {
                return Err(pending);
            }
            thread::sleep(CHECK_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
        }
    }

    fn obligations(&self) -> Vec<Obligation> {
        self.state
            .lock()
            .unwrap()
            .invocations
            .iter()
            .map(|entry| {
                let failure = entry.failure.lock().unwrap().clone();
                let native_finished = entry.worker.lock().unwrap().as_ref().map_or_else(
                    || entry.joined.load(Ordering::Acquire),
                    JoinHandle::is_finished,
                );
                Obligation {
                    invocation_id: entry.id,
                    owner_pid: std::process::id(),
                    failed_held: failure.is_some(),
                    native_finished,
                    failure_stage: failure.map(|failure| failure.stage),
                }
            })
            .collect()
    }
}

fn run_owned(invocation: &Invocation) -> WorkerResult {
    let mut scope = Process::new();
    let work = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if invocation.cancelled.load(Ordering::Acquire) {
            return Err(Failure::at("cancelled before process creation"));
        }
        scope.spawn(&invocation.source, &invocation.cancelled)?;
        let mut stdout = Vec::new();
        loop {
            if invocation.cancelled.load(Ordering::Acquire) {
                return Err(Failure::at("cancelled"));
            }
            scope.read_available(&mut stdout)?;
            match scope.exit_status()? {
                Some(code) if code != 0 => {
                    return Err(Failure {
                        stage: "exited unsuccessfully",
                        os_code: Some(code),
                    });
                }
                Some(_) if scope.output_closed() => {
                    return String::from_utf8(stdout)
                        .map(|value| value.trim().to_string())
                        .map_err(|_| Failure::at("output is not UTF-8"));
                }
                _ => thread::sleep(CHECK_INTERVAL),
            }
        }
    }));
    let outcome = work.unwrap_or_else(|_| Err(Failure::at("native owner panicked")));
    invocation.cleanup_limit();
    let cleanup = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scope.begin_cleanup()?;
        loop {
            if scope.poll_cleanup()? {
                if Instant::now() >= invocation.cleanup_limit() {
                    return Err(Failure::at("cleanup observation budget exhausted"));
                }
                scope.finalize()?;
                return if Instant::now() < invocation.cleanup_limit() {
                    Ok(())
                } else {
                    Err(Failure::at("resource finalization exceeded cleanup budget"))
                };
            }
            let deadline = invocation.cleanup_limit();
            if Instant::now() >= deadline {
                return Err(Failure::at("cleanup observation budget exhausted"));
            }
            thread::sleep(CHECK_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
        }
    }))
    .unwrap_or_else(|_| Err(Failure::at("native cleanup owner panicked")));
    match cleanup {
        Ok(()) => WorkerResult {
            scope,
            cleaned: true,
            outcome,
        },
        Err(error) => {
            invocation.record_failure(error.clone());
            WorkerResult {
                scope,
                cleaned: false,
                outcome: Err(error),
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Obligation {
    pub invocation_id: u64,
    pub owner_pid: u32,
    pub failed_held: bool,
    pub native_finished: bool,
    pub failure_stage: Option<&'static str>,
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Registry::new)
}

pub(super) async fn execute(command: &str) -> anyhow::Result<String> {
    registry().execute(command).await.map_err(Into::into)
}

pub(super) fn drain_before_exit() {
    if let Err(obligations) = registry().shutdown() {
        eprintln!(
            "[telex] credential cleanup FAILED_HELD in owning host {}: {:?}; \
             orderly exit is held. Operator intervention is required; \
             ownership must not be discarded or a clean exit claimed.",
            std::process::id(),
            obligations,
        );
        loop {
            thread::park();
        }
    }
}

pub(super) fn pending() -> Vec<Obligation> {
    registry().collect_finished();
    registry().obligations()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    fn complete_with(
        invocation: &Arc<Invocation>,
        barrier: Arc<Barrier>,
        cleaned: bool,
        outcome: Result<String, Failure>,
    ) {
        *invocation.worker.lock().unwrap() = Some(thread::spawn(move || {
            barrier.wait();
            WorkerResult {
                scope: Process::new(),
                cleaned,
                outcome,
            }
        }));
    }

    fn await_finished(invocation: &Invocation) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !invocation
            .worker
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .is_finished()
        {
            assert!(Instant::now() < deadline, "test owner did not finish");
            thread::yield_now();
        }
    }

    #[test]
    fn admission_counts_all_reserved_sources_without_starting_native_work() {
        let registry = Registry::new();
        let first = registry.reserve("source-a").unwrap().unwrap();
        assert!(registry.reserve("source-a").unwrap().is_none());
        let second = registry.reserve("source-b").unwrap().unwrap();
        assert!(registry.reserve("source-c").unwrap().is_none());
        first.cancel(Instant::now() + CLEANUP_BUDGET);
        assert!(registry.reserve("source-a").unwrap().is_none());
        assert!(registry.reserve("source-c").unwrap().is_none());
        assert!(first.worker.lock().unwrap().is_none());
        assert!(second.worker.lock().unwrap().is_none());
        assert_eq!(registry.obligations().len(), CAPACITY);
    }

    #[test]
    fn completed_native_work_keeps_source_and_slot_until_joined_receipt() {
        let registry = Registry::new();
        let invocation = registry.reserve("source-a").unwrap().unwrap();
        let barrier = Arc::new(Barrier::new(2));
        complete_with(&invocation, barrier.clone(), true, Ok("non-secret".into()));
        registry.collect_finished();
        assert!(registry.reserve("source-a").unwrap().is_none());
        barrier.wait();
        await_finished(&invocation);
        assert_eq!(registry.state.lock().unwrap().invocations.len(), 1);
        registry.collect_finished();
        assert!(invocation.joined.load(Ordering::Acquire));
        assert!(registry.obligations().is_empty());
        assert!(matches!(
            &*invocation.publication.lock().unwrap(),
            Publication::Completed(Some(Ok(value))) if value == "non-secret"
        ));
    }

    #[test]
    fn cancellation_wins_over_a_completed_unpublished_credential() {
        let registry = Registry::new();
        let invocation = registry.reserve("source-a").unwrap().unwrap();
        let barrier = Arc::new(Barrier::new(2));
        complete_with(
            &invocation,
            barrier.clone(),
            true,
            Ok("must-not-publish".into()),
        );
        invocation.cancel(Instant::now() + CLEANUP_BUDGET);
        barrier.wait();
        await_finished(&invocation);
        registry.collect_finished();
        assert!(matches!(
            *invocation.publication.lock().unwrap(),
            Publication::Cancelled
        ));
        assert!(registry.obligations().is_empty());
    }

    #[test]
    fn resource_close_and_join_precede_atomic_publication_and_source_release() {
        let registry = Arc::new(Registry::new());
        let invocation = registry.reserve("source-a").unwrap().unwrap();
        let worker_barrier = Arc::new(Barrier::new(2));
        let publication_gate = Arc::new(PublicationGate {
            after_close: Barrier::new(2),
            release: Barrier::new(2),
        });
        *invocation.publication_gate.lock().unwrap() = Some(publication_gate.clone());
        complete_with(
            &invocation,
            worker_barrier.clone(),
            true,
            Ok("must-not-publish".into()),
        );
        worker_barrier.wait();
        await_finished(&invocation);
        let collecting = {
            let registry = registry.clone();
            thread::spawn(move || registry.collect_finished())
        };
        publication_gate.after_close.wait();
        assert!(invocation.joined.load(Ordering::Acquire));
        assert!(matches!(
            *invocation.publication.lock().unwrap(),
            Publication::Pending
        ));
        assert!(registry.reserve("source-a").unwrap().is_none());
        invocation.cancel(Instant::now() + CLEANUP_BUDGET);
        publication_gate.release.wait();
        collecting.join().unwrap();
        assert!(matches!(
            *invocation.publication.lock().unwrap(),
            Publication::Cancelled
        ));
        assert!(registry.obligations().is_empty());
    }

    #[test]
    fn failed_receipt_retains_scope_source_capacity_and_named_obligation() {
        let registry = Registry::new();
        let invocation = registry.reserve("source-a").unwrap().unwrap();
        let barrier = Arc::new(Barrier::new(2));
        complete_with(
            &invocation,
            barrier.clone(),
            false,
            Err(Failure::at("injected missing receipt")),
        );
        barrier.wait();
        await_finished(&invocation);
        registry.collect_finished();
        assert!(invocation.held_scope.lock().unwrap().is_some());
        assert!(invocation.joined.load(Ordering::Acquire));
        assert!(registry.reserve("source-a").is_err());
        assert!(registry.reserve("independent-source").unwrap().is_some());
        assert!(registry.reserve("third-source").unwrap().is_none());
        let pending = registry.obligations();
        assert_eq!(pending.len(), CAPACITY);
        assert!(pending[0].failed_held);
        assert_eq!(pending[0].failure_stage, Some("injected missing receipt"));
    }

    #[test]
    fn late_cleaned_owner_cannot_erase_an_established_failure_hold() {
        let registry = Registry::new();
        let invocation = registry.reserve("source-a").unwrap().unwrap();
        let barrier = Arc::new(Barrier::new(2));
        complete_with(&invocation, barrier.clone(), true, Ok("ineligible".into()));
        invocation.record_failure(Failure::at("earlier shutdown failure"));
        barrier.wait();
        await_finished(&invocation);
        registry.collect_finished();
        assert!(matches!(
            *invocation.publication.lock().unwrap(),
            Publication::Pending
        ));
        let pending = registry.obligations();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].failed_held);
        assert!(pending[0].native_finished);
        assert!(registry.reserve("source-a").is_err());
    }

    #[test]
    fn repeated_cancellation_and_shutdown_never_renew_cleanup_budget() {
        let invocation = Registry::new().reserve("source").unwrap().unwrap();
        let original = Instant::now() + Duration::from_millis(20);
        invocation.cancel(original);
        invocation.cancel(Instant::now() + CLEANUP_BUDGET);
        assert_eq!(invocation.cleanup_limit(), original);
        let earlier = original - Duration::from_millis(10);
        invocation.limit_cleanup(earlier);
        assert_eq!(invocation.cleanup_limit(), earlier);
    }

    #[tokio::test]
    async fn queued_cancellation_creates_no_additional_owner() {
        let registry = Registry::new();
        let first = registry.reserve("first").unwrap().unwrap();
        let second = registry.reserve("second").unwrap().unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), registry.execute("never-started"),)
                .await
                .is_err()
        );
        assert_eq!(registry.state.lock().unwrap().next_id, 3);
        assert!(first.worker.lock().unwrap().is_none());
        assert!(second.worker.lock().unwrap().is_none());
    }

    #[test]
    fn shutdown_closes_admission_and_returns_failure_instead_of_false_receipt() {
        let registry = Registry::new();
        let invocation = registry.reserve("failed").unwrap().unwrap();
        *invocation.failure.lock().unwrap() = Some(Failure::at("test failure held"));
        invocation.joined.store(true, Ordering::Release);
        let pending = registry.shutdown().unwrap_err();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].failed_held);
        assert!(registry.reserve("new-source").is_err());
        assert_eq!(registry.state.lock().unwrap().invocations.len(), 1);
    }

    #[test]
    fn entirely_failed_held_capacity_rejects_new_sources_without_discarding_owners() {
        let registry = Registry::new();
        for source in ["first", "second"] {
            let invocation = registry.reserve(source).unwrap().unwrap();
            invocation.record_failure(Failure::at("injected held cleanup"));
        }
        assert!(matches!(
            registry.reserve("third"),
            Err(Failure {
                stage: "admission held by unresolved cleanup obligations",
                ..
            })
        ));
        assert_eq!(registry.state.lock().unwrap().invocations.len(), CAPACITY);
        assert_eq!(registry.state.lock().unwrap().next_id, 3);
    }

    #[test]
    fn ownerless_shutdown_does_not_spend_a_cleanup_budget() {
        let registry = Registry::new();
        let started = Instant::now();
        assert!(registry.shutdown().is_ok());
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    #[ignore = "isolated failed-held host entrypoint"]
    fn failed_held_host_fixture() {
        let invocation = registry()
            .reserve("non-secret-test-source")
            .unwrap()
            .unwrap();
        invocation.joined.store(true, Ordering::Release);
        invocation.record_failure(Failure::at("injected scope observation failure"));
        drain_before_exit();
        panic!("FAILED_HELD must not become a clean exit");
    }

    #[test]
    fn failed_held_shutdown_keeps_the_owning_host_alive_and_reports_its_obligation() {
        use std::fs::{self, File};
        use std::process::{Command, Stdio};
        let root = std::env::temp_dir().join(format!(
            "telex-credential-held-{}-{}",
            std::process::id(),
            crate::model::now_ms(),
        ));
        fs::create_dir(&root).unwrap();
        let stderr = root.join("stderr");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "profiles::password_command::tests::failed_held_host_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("TELEX_HOME", root.join("home"))
            .env("TELEX_DB", root.join("unused.db"))
            .env("TELEX_CONFIG", root.join("config.toml"))
            .env("TELEX_INSTALL_ROOT", root.join("install"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(File::create(&stderr).unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let observed = loop {
            let text = fs::read_to_string(&stderr).unwrap();
            if text.contains("orderly exit is held") {
                break text.contains("invocation_id: 1")
                    && text.contains("injected scope observation failure")
                    && child.try_wait().unwrap().is_none();
            }
            if Instant::now() >= deadline || child.try_wait().unwrap().is_some() {
                break false;
            }
            thread::sleep(CHECK_INTERVAL);
        };
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
        }
        child.wait().unwrap();
        assert!(
            observed,
            "failed-held host did not retain/report its obligation"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
