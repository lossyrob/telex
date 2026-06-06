//! Telex: a CLI-first message fabric for AI agent sessions.

pub mod backend;
pub mod cli;
pub mod commands;
pub mod config;
#[cfg(feature = "entra")]
pub mod credential;
pub mod ipc;
pub mod model;
pub mod output;
pub mod profiles;

#[cfg(not(any(feature = "sqlite", feature = "postgres")))]
compile_error!("enable at least one backend feature: `sqlite` and/or `postgres`");
