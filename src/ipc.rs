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
    Keepalive { heartbeat_age_ms: i64 },
    Pong {
        heartbeat_age_ms: i64,
        /// The address this holder serves. Lets a `ping` confirm it reached the holder for the
        /// address it asked about, not a different holder whose endpoint name happens to collide
        /// under the lossy `sanitize()`. `default` for frames from older holders that omit it.
        #[serde(default)]
        served_address: Option<String>,
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

/// The platform IPC endpoint name for an address (named pipe on Windows, socket path on Unix).
/// Best-effort, for the holder registry's `socket` field and debugging.
pub fn endpoint(address: &str) -> Option<String> {
    #[cfg(windows)]
    {
        Some(pipe_name(address))
    }
    #[cfg(unix)]
    {
        socket_path(address)
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    }
}

/// Liveness probe: connect to the holder for `address`, send `Ping`, and confirm a `Pong` whose
/// `served_address` matches. The whole connect+write+read is bounded by a short timeout so a stale
/// endpoint (dead/hung holder) fails fast and never stalls a caller (e.g. `send`'s `from`
/// resolution). Returns `false` on any error, timeout, or address mismatch.
pub async fn ping(address: &str) -> bool {
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let address = address.to_string();
    let probe = async move {
        let stream = connect(&address).await.ok()?;
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut line = serde_json::to_string(&Request::Ping).ok()?;
        line.push('\n');
        write_half.write_all(line.as_bytes()).await.ok()?;
        write_half.flush().await.ok()?;

        let mut reader = BufReader::new(read_half);
        let mut buf = String::new();
        if reader.read_line(&mut buf).await.ok()? == 0 {
            return None;
        }
        match serde_json::from_str::<Frame>(buf.trim()).ok()? {
            Frame::Pong {
                served_address: Some(served),
                ..
            } => Some(served == address),
            _ => Some(false),
        }
    };

    matches!(
        tokio::time::timeout(Duration::from_millis(250), probe).await,
        Ok(Some(true))
    )
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    /// Module-wide lock for tests that mutate the process-global `TELEX_HOME` (Unix endpoint paths
    /// derive from it). Rust runs tests in parallel threads in one binary, so any test touching
    /// `TELEX_HOME` must hold this first. No `serial_test` dev-dep is available.
    pub static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Acquire `ENV_LOCK`, tolerating poisoning from a panicking earlier test.
    pub fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Spawn a minimal holder that answers each connection's `Ping` with a `Pong` stamped with
    /// `served_address`, so `ipc::ping` has a real endpoint to probe in tests. Abort the returned
    /// handle to stop it.
    pub fn spawn_pong_holder(address: &str) -> tokio::task::JoinHandle<()> {
        let mut listener = Listener::bind(address).expect("bind test holder");
        let served = address.to_string();
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok(conn) => {
                        let served = served.clone();
                        tokio::spawn(async move {
                            let (read_half, mut write_half) = tokio::io::split(conn);
                            let mut reader = BufReader::new(read_half);
                            let mut line = String::new();
                            if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                                return;
                            }
                            let frame = Frame::Pong {
                                heartbeat_age_ms: 0,
                                served_address: Some(served),
                            };
                            if let Ok(mut s) = serde_json::to_string(&frame) {
                                s.push('\n');
                                let _ = write_half.write_all(s.as_bytes()).await;
                                let _ = write_half.flush().await;
                            }
                        });
                    }
                    Err(_) => return,
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pong_serde_tolerates_missing_served_address() {
        // Frames from an older holder omit served_address; it must default to None.
        let f: Frame = serde_json::from_str(r#"{"type":"pong","heartbeat_age_ms":5}"#).unwrap();
        match f {
            Frame::Pong {
                heartbeat_age_ms,
                served_address,
            } => {
                assert_eq!(heartbeat_age_ms, 5);
                assert_eq!(served_address, None);
            }
            _ => panic!("expected pong"),
        }
    }

    #[test]
    fn sanitize_maps_unsafe_chars_to_underscore() {
        assert_eq!(sanitize("impl:tx-2026:issue-4"), "impl_tx-2026_issue-4");
    }
}
