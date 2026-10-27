//! Telex Watcher v2: trusted local observation, fixed routing, and receipt-gated state.
//!
//! [`Runtime`] is the sole registry owner. [`management::LocalManagement`] is
//! the in-process administration boundary carried by the same-user local
//! transport and CLI. Detector/provider policy belongs outside this crate.

pub mod cli;
mod client;
pub mod evidence;
pub mod management;
mod ownership;
pub mod presentation;
pub mod process;
pub mod protocol_v2;
mod recovery;
mod registry_v2;
mod runtime;
mod staging;
pub mod transport;

pub use registry_v2::{ManagementAudit, Mutation, Refusal};
pub use runtime::{Runtime, RuntimeConfig};

#[cfg(all(test, any(windows, target_os = "linux")))]
mod runtime_tests;

#[cfg(all(test, any(windows, target_os = "linux")))]
mod runtime_resilience_tests;

#[cfg(all(test, any(windows, target_os = "linux")))]
#[path = "../../tests/support/telex_isolation.rs"]
mod isolation;
