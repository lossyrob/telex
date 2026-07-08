//! Telex: a CLI-first message fabric for AI agent sessions.

pub mod backend;
pub mod cli;
pub mod commands;
pub mod config;
#[cfg(feature = "entra")]
pub mod credential;
pub mod daemon;
pub mod daemon_ipc;
pub mod identity;
pub mod install;
// Legacy resident-holder IPC/registry surface retained for compatibility with
// pre-daemon commands and tests. New membership and delivery flows should use
// `daemon`/`daemon_ipc`.
pub mod ipc;
pub mod model;
pub mod output;
pub mod profiles;
// In-binary release upgrade (`telex upgrade` with no --from): discover a GitHub release,
// download + verify + extract the platform asset. Compiled only with the `self-update` feature.
#[cfg(feature = "self-update")]
pub mod release;
// Legacy address-keyed holder registry; daemon singleton status is exposed via
// `daemon`.
pub mod registry;
pub mod session_watch;

#[cfg(not(any(feature = "sqlite", feature = "postgres")))]
compile_error!("enable at least one backend feature: `sqlite` and/or `postgres`");
