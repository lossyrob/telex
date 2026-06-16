//! Local IPC between the resident holder (`attach`) and the ephemeral delivery client
//! (`wait`). Named pipe on Windows, unix domain socket elsewhere. The frame protocol is
//! one JSON object per line. Delivery is the exit trigger: handing the client a Message
//! frame is the instruction to print it and exit.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Request the waiter sends to the holder.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Block until an actionable message is available (or the holder times out).
    Wait {
        address: String,
        #[serde(default)]
        since: i64,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    /// Liveness probe; the holder answers immediately with Pong.
    Ping,
    /// Ask the holder to release its lease and exit.
    Shutdown,
}

/// Frames the holder writes to the waiter. One per line.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Frame {
    /// Periodic "I'm alive" carrying the age of the holder's last DB heartbeat.
    Keepalive {
        heartbeat_age_ms: i64,
    },
    Pong {
        heartbeat_age_ms: i64,
    },
    /// Acknowledge a Shutdown request before the holder exits.
    ShuttingDown,
    /// Delivery — the waiter prints this and exits 0.
    Message {
        id: i64,
        thread_id: i64,
        parent_id: Option<i64>,
        from_addr: Option<String>,
        to_addr: String,
        kind: String,
        attention: String,
        requires_disposition: bool,
        subject: Option<String>,
        body: String,
        sent_at_ms: i64,
        buffered_at_ms: i64,
    },
    /// Holder reached the waiter's requested idle timeout with no message.
    Timeout,
}

/// Sanitize an address into an IPC-safe token.
pub fn sanitize(address: &str) -> String {
    let mut s: String = address
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.len() > 120 {
        s.truncate(120);
    }
    s
}

#[cfg(windows)]
pub fn pipe_name(address: &str) -> String {
    format!(r"\\.\pipe\telex-{}", sanitize(address))
}

#[cfg(unix)]
pub fn socket_path(address: &str) -> Result<std::path::PathBuf> {
    Ok(crate::config::run_dir()?.join(format!("{}.sock", sanitize(address))))
}

// ----------------------------- Windows -----------------------------
#[cfg(windows)]
mod platform {
    use super::*;
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

    pub type Conn = NamedPipeServer;

    pub struct Listener {
        pipe_name: String,
        next: Option<NamedPipeServer>,
    }

    impl Listener {
        pub fn bind(address: &str) -> Result<Self> {
            let pipe_name = pipe_name(address);
            let server = ServerOptions::new()
                .first_pipe_instance(true)
                .create(&pipe_name)
                .with_context(|| format!("creating named pipe {pipe_name}"))?;
            Ok(Self {
                pipe_name,
                next: Some(server),
            })
        }

        pub async fn accept(&mut self) -> Result<Conn> {
            let server = self.next.take().expect("pipe instance present");
            server.connect().await.context("waiting for pipe client")?;
            // Pre-create the next instance so a subsequent client can connect.
            self.next = Some(ServerOptions::new().create(&self.pipe_name)?);
            Ok(server)
        }
    }

    /// Connect a client to the holder's pipe, retrying briefly on ERROR_PIPE_BUSY.
    pub async fn connect(
        address: &str,
    ) -> Result<tokio::net::windows::named_pipe::NamedPipeClient> {
        let name = pipe_name(address);
        const ERROR_PIPE_BUSY: i32 = 231;
        for _ in 0..20 {
            match ClientOptions::new().open(&name) {
                Ok(client) => return Ok(client),
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                Err(e) => return Err(e).with_context(|| format!("connecting to {name}")),
            }
        }
        anyhow::bail!("pipe {name} stayed busy")
    }
}

// ------------------------------ Unix -------------------------------
#[cfg(unix)]
mod platform {
    use super::*;
    use tokio::net::{UnixListener, UnixStream};

    pub type Conn = UnixStream;

    pub struct Listener {
        inner: UnixListener,
        path: std::path::PathBuf,
    }

    impl Listener {
        pub fn bind(address: &str) -> Result<Self> {
            let path = socket_path(address)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            // Remove a stale socket file from a previous holder.
            let _ = std::fs::remove_file(&path);
            let inner = UnixListener::bind(&path)
                .with_context(|| format!("binding unix socket {}", path.display()))?;
            Ok(Self { inner, path })
        }

        pub async fn accept(&mut self) -> Result<Conn> {
            let (stream, _) = self.inner.accept().await.context("accepting unix client")?;
            Ok(stream)
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    pub async fn connect(address: &str) -> Result<UnixStream> {
        let path = socket_path(address)?;
        UnixStream::connect(&path)
            .await
            .with_context(|| format!("connecting to {}", path.display()))
    }
}

pub use platform::{connect, Conn, Listener};
