//! Telex: a CLI-first message fabric for AI agent sessions.

pub mod backend;
pub mod cli;
pub mod commands;
pub mod config;
#[cfg(feature = "entra")]
pub mod credential;
pub mod daemon;
pub mod daemon_ipc;
/// Daemon-owned station-intent reconciliation (ADR 0050). Physically `src/daemon_reconcile.rs`,
/// mounted inside `daemon` so it can use the daemon's private state without widening that surface
/// to the whole crate, and re-exported here at the path the rest of the codebase refers to.
pub use crate::daemon::reconcile as daemon_reconcile;
/// Generic handler-kind / producer-root registry and the single shared push argv builder.
pub mod handler_kinds;
pub mod identity;
pub mod install;
/// Test harness for the station-intent path: a controllable fake producer endpoint and intent
/// fixtures. Compiled with the crate (like `daemon::test_support`) so integration tests can use it.
#[doc(hidden)]
pub mod intent_test_support;
// Legacy resident-holder IPC/registry surface retained for compatibility with
// pre-daemon commands and tests. New membership and delivery flows should use
// `daemon`/`daemon_ipc`.
pub mod ipc;
pub mod model;
pub mod output;
/// Shared owner-private filesystem and process-identity primitives (see ADR 0025, ADR 0050).
pub mod platform_fs;
pub mod profiles;
// In-binary release upgrade (`telex upgrade` with no --from): discover a GitHub release,
// download + verify + extract the platform asset. Compiled only with the `self-update` feature.
#[cfg(feature = "self-update")]
pub mod release;
// Legacy address-keyed holder registry; daemon singleton status is exposed via
// `daemon`.
pub mod registry;
pub mod session_watch;
/// Durable, host-local, owner-private station-intent records (ADR 0050).
pub mod station_intent;

#[cfg(not(any(feature = "sqlite", feature = "postgres")))]
compile_error!("enable at least one backend feature: `sqlite` and/or `postgres`");
