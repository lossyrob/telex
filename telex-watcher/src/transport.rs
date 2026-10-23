//! Versioned, same-user local administration of an already-owned registry.
//!
//! Hosts must call [`Server::bind`] only after `Runtime::open` acquires registry
//! ownership. Run the server in a task; when the runtime stops, abort **and
//! await** that task before releasing the ownership lifetime. Dropping the run
//! future synchronously closes its listener and bounded client futures. Unix removes
//! only its own socket inode; Windows closes all named-pipe instances. A crash
//! leaves at most an inert Unix socket, which the next owner can reclaim.
//!
//! Frames are a four-byte big-endian byte length followed by one JSON object.
//! There is one command per connection, with no streaming or remote transport.
//! Registration JSON remains an exact byte vector, including invalid UTF-8 or
//! duplicate keys, so only the existing registration parser decides admission.

use crate::management::{Command, LocalManagement, Response};
use futures_util::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;

pub const PROTOCOL_VERSION: u16 = 1;
/// Allows a 2 MiB registration even with JSON's byte-array encoding overhead.
pub const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CLIENTS: usize = 16;
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
pub const EXECUTE_TIMEOUT: Duration = Duration::from_secs(30);
pub const RESPONSE_TIMEOUT: Duration = Duration::from_secs(41);
const BUSY_WRITE_TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransportErrorCode {
    RuntimeNotRunning,
    Timeout,
    OutcomeUnknown,
    Unauthorized,
    InvalidRequest,
    Busy,
    ResponseTooLarge,
    RuntimeError,
    Io,
}

impl TransportErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeNotRunning => "runtime-not-running",
            Self::Timeout => "timeout",
            Self::OutcomeUnknown => "outcome-unknown",
            Self::Unauthorized => "unauthorized",
            Self::InvalidRequest => "invalid-request",
            Self::Busy => "busy",
            Self::ResponseTooLarge => "response-too-large",
            Self::RuntimeError => "runtime-error",
            Self::Io => "io",
        }
    }
}

impl std::fmt::Display for TransportErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Messages deliberately omit registry contents and underlying client errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportError {
    pub code: TransportErrorCode,
    pub message: String,
}

impl TransportError {
    pub fn code(&self) -> &'static str {
        self.code.as_str()
    }

    fn new(code: TransportErrorCode, message: &'static str) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn outcome_unknown() -> Self {
        Self::new(
            TransportErrorCode::OutcomeUnknown,
            "The command may have been admitted; inspect its result before retrying.",
        )
    }
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for TransportError {}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestFrame {
    version: u16,
    command: Command,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseFrame {
    version: u16,
    result: Result<Response, TransportError>,
}

#[derive(Clone, Copy)]
struct Limits {
    read: Duration,
    write: Duration,
    execute: Duration,
    response: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            read: READ_TIMEOUT,
            write: WRITE_TIMEOUT,
            execute: EXECUTE_TIMEOUT,
            response: RESPONSE_TIMEOUT,
        }
    }
}

pub struct Server {
    listener: platform::Listener,
    management: LocalManagement,
    limits: Limits,
}

impl Server {
    /// Synchronous local bind inside a Tokio I/O runtime. The caller must
    /// already own this registry.
    pub fn bind(registry: &Path, management: LocalManagement) -> anyhow::Result<Self> {
        Ok(Self {
            listener: platform::Listener::bind(&endpoint_key(registry)?)?,
            management,
            limits: Limits::default(),
        })
    }

    /// Runs until cancelled, a listener failure, or the Shutdown reply has been
    /// written (bounded by WRITE_TIMEOUT). Other client errors are isolated.
    pub async fn run(mut self) -> anyhow::Result<()> {
        // These are owned futures, not spawned tasks: cancellation drops every
        // connection synchronously, before the registry ownership can end.
        let mut clients = FuturesUnordered::new();
        loop {
            tokio::select! {
                biased;
                Some(stopping) = clients.next(), if !clients.is_empty() => {
                    if stopping {
                        return Ok(());
                    }
                }
                connection = self.listener.accept() => {
                    let mut connection = connection?;
                    let authorization = platform::verify_client(&connection);
                    let rejection = authorization.err().or_else(|| {
                        (clients.len() >= MAX_CLIENTS).then(|| TransportError::new(
                            TransportErrorCode::Busy,
                            "The management endpoint has reached its concurrent-client limit.",
                        ))
                    });
                    if let Some(error) = rejection {
                        let _ = timeout(
                            BUSY_WRITE_TIMEOUT,
                            send_result(&mut connection, Err(error)),
                        ).await;
                    } else {
                        let management = self.management.clone();
                        let limits = self.limits;
                        clients.push(async move {
                            serve(&mut connection, management, limits).await
                        });
                    }
                }
            }
        }
    }
}

/// Connects only to the current user's registry owner. Never opens SQLite,
/// acquires registry ownership, starts a runtime, or falls back to offline work.
pub async fn execute(registry: &Path, command: Command) -> Result<Response, TransportError> {
    execute_with_limits(registry, command, Limits::default()).await
}

async fn execute_with_limits(
    registry: &Path,
    command: Command,
    limits: Limits,
) -> Result<Response, TransportError> {
    let frame = RequestFrame {
        version: PROTOCOL_VERSION,
        command,
    };
    let bytes = encode(&frame, MAX_REQUEST_BYTES).map_err(|_| {
        TransportError::new(
            TransportErrorCode::InvalidRequest,
            "The command cannot be encoded within the management request limit.",
        )
    })?;
    let key = endpoint_key(registry)?;
    let mut connection = timeout(CONNECT_TIMEOUT, platform::connect(&key))
        .await
        .map_err(|_| {
            TransportError::new(
                TransportErrorCode::RuntimeNotRunning,
                "No reachable owning runtime answered within the connection deadline.",
            )
        })??;
    platform::verify_server(&connection)?;
    // After the first write, an absent reply cannot prove non-admission.
    timeout(limits.write, write_frame(&mut connection, &bytes))
        .await
        .map_err(|_| TransportError::outcome_unknown())?
        .map_err(|_| TransportError::outcome_unknown())?;
    let bytes = timeout(
        limits.response,
        read_frame(&mut connection, MAX_RESPONSE_BYTES),
    )
    .await
    .map_err(|_| TransportError::outcome_unknown())?
    .map_err(|_| TransportError::outcome_unknown())?;
    let response: ResponseFrame = serde_json::from_slice(&bytes).map_err(|_| {
        TransportError::new(
            TransportErrorCode::OutcomeUnknown,
            "The runtime reply was invalid; inspect the command result before retrying.",
        )
    })?;
    if response.version != PROTOCOL_VERSION {
        return Err(TransportError::new(
            TransportErrorCode::OutcomeUnknown,
            "The runtime reply version is unsupported; inspect before retrying.",
        ));
    }
    response.result
}

async fn serve<S>(connection: &mut S, management: LocalManagement, limits: Limits) -> bool
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let request = match timeout(limits.read, read_request(connection)).await {
        Ok(result) => result,
        Err(_) => Err(TransportError::new(
            TransportErrorCode::Timeout,
            "The request read deadline expired before admission.",
        )),
    };
    let shutdown_requested = matches!(&request, Ok(Command::Shutdown));
    let result = match request {
        Err(error) => Err(error),
        Ok(command) => match timeout(limits.execute, management.execute(command)).await {
            Err(_) => Err(TransportError::outcome_unknown()),
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) if error.to_string() == "watcher-owner-unavailable" => {
                Err(TransportError::new(
                    TransportErrorCode::RuntimeNotRunning,
                    "The registry owner is no longer running.",
                ))
            }
            Ok(Err(error)) if error.to_string() == "watcher-owner-outcome-unknown" => {
                Err(TransportError::outcome_unknown())
            }
            Ok(Err(_)) => Err(TransportError::new(
                TransportErrorCode::RuntimeError,
                "The registry owner could not complete the command.",
            )),
        },
    };
    let stopping = shutdown_requested && matches!(&result, Ok(Response::Stopping));
    let _ = timeout(limits.write, send_result(connection, result)).await;
    stopping
}

async fn read_request<S: AsyncRead + Unpin>(connection: &mut S) -> Result<Command, TransportError> {
    let bytes = read_frame(connection, MAX_REQUEST_BYTES)
        .await
        .map_err(|_| {
            TransportError::new(
                TransportErrorCode::InvalidRequest,
                "The management request frame is invalid or exceeds the byte limit.",
            )
        })?;
    // Validate opaque state before serde can collapse duplicate keys or round
    // out-of-model numeric tokens. Response evidence keeps its typed serde path.
    let value = crate::protocol_v2::parse_json(&bytes).map_err(|_| {
        TransportError::new(
            TransportErrorCode::InvalidRequest,
            "The request must be strict I-JSON without duplicate object members.",
        )
    })?;
    let frame: RequestFrame = serde_json::from_value(value).map_err(|_| {
        TransportError::new(
            TransportErrorCode::InvalidRequest,
            "The management request must be one valid typed JSON command.",
        )
    })?;
    if frame.version != PROTOCOL_VERSION {
        return Err(TransportError::new(
            TransportErrorCode::InvalidRequest,
            "The management protocol version is unsupported.",
        ));
    }
    Ok(frame.command)
}

async fn send_result<S: AsyncWrite + Unpin>(
    connection: &mut S,
    result: Result<Response, TransportError>,
) -> io::Result<()> {
    let frame = ResponseFrame {
        version: PROTOCOL_VERSION,
        result,
    };
    let bytes = encode(&frame, MAX_RESPONSE_BYTES).or_else(|_| {
        encode(
            &ResponseFrame {
                version: PROTOCOL_VERSION,
                result: Err(TransportError::new(
                    TransportErrorCode::ResponseTooLarge,
                    "The command completed but its response exceeds the byte limit; narrow the query and inspect before retrying mutations.",
                )),
            },
            MAX_RESPONSE_BYTES,
        )
    })?;
    write_frame(connection, &bytes).await
}

fn endpoint_key(registry: &Path) -> Result<String, TransportError> {
    let identity =
        crate::ownership::physical_identity(registry).map_err(|error| connect_error(&error))?;
    Ok(hash_key(identity.as_bytes()))
}

fn hash_key(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    let mut hash = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(hash, "{byte:02x}").expect("writing to a String cannot fail");
    }
    hash
}

fn connect_error(error: &io::Error) -> TransportError {
    let (code, message) = match error.kind() {
        io::ErrorKind::PermissionDenied => (
            TransportErrorCode::Unauthorized,
            "The registry identity or management endpoint is not accessible to this user.",
        ),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => (
            TransportErrorCode::RuntimeNotRunning,
            "No owning runtime is listening for this registry.",
        ),
        _ => (
            TransportErrorCode::Io,
            "The local management endpoint could not be accessed.",
        ),
    };
    TransportError::new(code, message)
}

async fn read_frame<S: AsyncRead + Unpin>(stream: &mut S, cap: usize) -> io::Result<Vec<u8>> {
    let length = stream.read_u32().await? as usize;
    if length == 0 || length > cap {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame byte limit",
        ));
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await?;
    Ok(bytes)
}

async fn write_frame<S: AsyncWrite + Unpin>(stream: &mut S, bytes: &[u8]) -> io::Result<()> {
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(bytes).await?;
    stream.flush().await
}

fn encode<T: Serialize>(value: &T, cap: usize) -> io::Result<Vec<u8>> {
    struct Bounded {
        bytes: Vec<u8>,
        cap: usize,
    }
    impl io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.cap.saturating_sub(self.bytes.len()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "frame byte limit",
                ));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Bounded {
        bytes: Vec::new(),
        cap,
    };
    serde_json::to_writer(&mut writer, value).map_err(io::Error::other)?;
    Ok(writer.bytes)
}

#[cfg(test)]
mod tests;
