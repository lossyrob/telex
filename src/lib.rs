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
pub mod ipc;
pub mod model;
pub mod output;
pub mod profiles;
pub mod registry;
pub mod session_watch;

#[cfg(not(any(feature = "sqlite", feature = "postgres")))]
compile_error!("enable at least one backend feature: `sqlite` and/or `postgres`");
