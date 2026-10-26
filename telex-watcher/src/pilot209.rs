//! Issue #209 bounded hosted pilot: test-build-only phase recorder.
//!
//! Non-merging diagnostic branch. Compiled only under `cfg(test)` and active
//! only on a thread where the opaque management test called [`begin`] because
//! `PILOT209_OUT` is set. All clocks are monotonic `Instant`s relative to the
//! per-state epoch. SQLite statement events (`sql:*`) come from the legacy
//! `sqlite3_profile` hook only: `elapsed_us` is null and `sqlite_us` is
//! SQLite's own coarse reported duration. A `COMMIT`
//! or autocommit write duration is a flush proxy (journal I/O plus
//! FlushFileBuffers plus journal delete), not a raw fsync measurement.
//! Callbacks run inside C frames, so nothing here may panic.

use std::cell::RefCell;
use std::time::{Duration, Instant};

struct Event {
    phase: String,
    start_us: u64,
    elapsed_us: Option<u64>,
    sqlite_us: Option<u64>,
}

struct Recorder {
    epoch: Instant,
    events: Vec<Event>,
}

thread_local! {
    static RECORDER: RefCell<Option<Recorder>> = const { RefCell::new(None) };
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn with_recorder(f: impl FnOnce(&mut Recorder)) {
    let _ = RECORDER.try_with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut() {
            if let Some(recorder) = slot.as_mut() {
                f(recorder);
            }
        }
    });
}

/// Output directory for pilot records; `None` leaves the test unchanged.
pub(crate) fn output_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("PILOT209_OUT").map(std::path::PathBuf::from)
}

pub(crate) fn begin() {
    let _ = RECORDER.try_with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut() {
            *slot = Some(Recorder {
                epoch: Instant::now(),
                events: Vec::new(),
            });
        }
    });
}

pub(crate) fn active() -> bool {
    RECORDER
        .try_with(|cell| cell.try_borrow().map(|slot| slot.is_some()).unwrap_or(false))
        .unwrap_or(false)
}

/// Records `phase` as starting at `started` and ending now.
pub(crate) fn record(phase: &str, started: Instant) {
    let now = Instant::now();
    with_recorder(|recorder| {
        recorder.events.push(Event {
            phase: phase.to_owned(),
            start_us: micros(started.saturating_duration_since(recorder.epoch)),
            elapsed_us: Some(micros(now.saturating_duration_since(started))),
            sqlite_us: None,
        });
    });
}

/// Stops recording on this thread and returns the events as JSON.
pub(crate) fn end() -> serde_json::Value {
    let recorder = RECORDER
        .try_with(|cell| cell.try_borrow_mut().ok().and_then(|mut slot| slot.take()))
        .ok()
        .flatten();
    let events = recorder.map(|recorder| recorder.events).unwrap_or_default();
    serde_json::Value::Array(
        events
            .into_iter()
            .map(|event| {
                serde_json::json!({
                    "phase": event.phase,
                    "start_us": event.start_us,
                    "elapsed_us": event.elapsed_us,
                    "sqlite_us": event.sqlite_us,
                })
            })
            .collect(),
    )
}

pub(crate) fn command_kind(command: &crate::management::Command) -> &'static str {
    use crate::management::Command;
    match command {
        Command::Register { .. } => "register",
        Command::Show { .. } => "show",
        Command::Remove { .. } => "remove",
        Command::Attempts { .. } => "attempts",
        Command::Shutdown => "shutdown",
        _ => "other",
    }
}

/// Hooks statement timing onto a connection opened on a recording thread.
///
/// Profile only: SQLite's `sqlite3_profile` clears the legacy trace flag, so
/// the two legacy hooks cannot be combined. The event's end is the monotonic
/// callback time; its duration is SQLite's own reported value (wall-clock,
/// coarse, first step to reset/finalize), so `start_us` is an estimate.
pub(crate) fn attach(connection: &mut rusqlite::Connection) {
    if active() {
        connection.profile(Some(sql_finished));
    }
}

fn label(sql: &str) -> String {
    let collapsed = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(40).collect()
}

fn sql_finished(sql: &str, reported: Duration) {
    let now = Instant::now();
    let label = label(sql);
    with_recorder(|recorder| {
        recorder.events.push(Event {
            phase: format!("sql:{label}"),
            start_us: micros(
                now.saturating_duration_since(recorder.epoch)
                    .saturating_sub(reported),
            ),
            elapsed_us: None,
            sqlite_us: Some(micros(reported)),
        });
    });
}