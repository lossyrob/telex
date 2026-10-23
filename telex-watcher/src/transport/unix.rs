use super::{connect_error, TransportError, TransportErrorCode};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use tokio::net::{UnixListener, UnixStream};

pub(super) struct Listener {
    inner: UnixListener,
    path: PathBuf,
    identity: (u64, u64),
}

impl Listener {
    pub(super) fn bind(key: &str) -> anyhow::Result<Self> {
        let path = endpoint(key, true)?;
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            validate_socket(&metadata)?;
            if socket_is_live(&path)? {
                anyhow::bail!("management-endpoint-already-bound");
            }
            // Do not unlink a different inode if the path changed after probing.
            if same_inode(&path, (metadata.dev(), metadata.ino())) {
                std::fs::remove_file(&path)?;
            } else {
                anyhow::bail!("management-endpoint-changed-during-bind");
            }
        }
        let inner = UnixListener::bind(&path)?;
        let metadata = std::fs::symlink_metadata(&path)?;
        let listener = Self {
            inner,
            path,
            identity: (metadata.dev(), metadata.ino()),
        };
        std::fs::set_permissions(&listener.path, std::fs::Permissions::from_mode(0o600))?;
        Ok(listener)
    }

    pub(super) async fn accept(&mut self) -> io::Result<UnixStream> {
        self.inner.accept().await.map(|(stream, _)| stream)
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if same_inode(&self.path, self.identity) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn same_inode(path: &Path, identity: (u64, u64)) -> bool {
    std::fs::symlink_metadata(path)
        .is_ok_and(|metadata| (metadata.dev(), metadata.ino()) == identity)
}

fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "management endpoint is not same-user private",
    )
}

fn validate_socket(metadata: &std::fs::Metadata) -> io::Result<()> {
    if !metadata.file_type().is_socket()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(denied());
    }
    Ok(())
}

/// The daemon's owner/0700 posture, but create-strict/validate-existing:
/// management must never chmod an existing user directory.
fn private_directory(path: &Path, create: bool) -> io::Result<()> {
    validate_ancestors(path)?;
    if create {
        match std::fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(denied());
    }
    Ok(())
}

fn validate_ancestors(path: &Path) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(denied());
    }
    let uid = unsafe { libc::geteuid() };
    for ancestor in path.ancestors().skip(1) {
        let metadata = std::fs::symlink_metadata(ancestor)?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || !matches!(metadata.uid(), owner if owner == 0 || owner == uid)
            || metadata.mode() & 0o022 != 0
        {
            return Err(denied());
        }
    }
    Ok(())
}

fn endpoint(key: &str, create: bool) -> Result<PathBuf, TransportError> {
    let home = dirs::home_dir().ok_or_else(|| {
        TransportError::new(
            TransportErrorCode::Io,
            "A local home directory is required for the management endpoint.",
        )
    })?;
    let root = home.join(".telex-watcher");
    private_directory(&root, create).map_err(|error| connect_error(&error))?;
    let path = root.join(key);
    // Both sockaddr_un.sun_path and Tokio's bind reject overlong paths.
    let address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if path.as_os_str().as_bytes().len() >= address.sun_path.len() {
        return Err(TransportError::new(
            TransportErrorCode::Io,
            "The home directory path is too long for a Unix management socket.",
        ));
    }
    Ok(path)
}

pub(super) async fn connect(key: &str) -> Result<UnixStream, TransportError> {
    let path = endpoint(key, false)?;
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| connect_error(&error))?;
    validate_socket(&metadata).map_err(|error| connect_error(&error))?;
    UnixStream::connect(path)
        .await
        .map_err(|error| connect_error(&error))
}

pub(super) fn verify_client(stream: &UnixStream) -> Result<(), TransportError> {
    let credentials = stream.peer_cred().map_err(|error| connect_error(&error))?;
    if credentials.uid() != unsafe { libc::geteuid() } {
        return Err(TransportError::new(
            TransportErrorCode::Unauthorized,
            "The local peer belongs to a different OS user.",
        ));
    }
    Ok(())
}

pub(super) fn verify_server(stream: &UnixStream) -> Result<(), TransportError> {
    verify_client(stream)
}

fn socket_is_live(path: &Path) -> io::Result<bool> {
    // A blocking connect could hang on a full listen backlog. Probe without
    // blocking; an in-progress or backlogged listener is never reclaimed.
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let socket = unsafe { OwnedFd::from_raw_fd(raw) };
    if unsafe { libc::fcntl(socket.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0
        || unsafe { libc::fcntl(socket.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= address.sun_path.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "socket path limit",
        ));
    }
    for (target, byte) in address.sun_path.iter_mut().zip(bytes) {
        *target = *byte as libc::c_char;
    }
    let length = std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1;
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    {
        address.sun_len = length as u8;
    }
    let result = unsafe {
        libc::connect(
            socket.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            length as libc::socklen_t,
        )
    };
    if result == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ECONNREFUSED) => Ok(false),
        Some(libc::EINPROGRESS | libc::EAGAIN | libc::EALREADY) => Ok(true),
        _ => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_directory_permissions_and_symlink_are_never_repaired() {
        let root = std::env::current_dir()
            .unwrap()
            .join(format!(".transport-unix-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            private_directory(&root, true).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(std::fs::metadata(&root).unwrap().mode() & 0o777, 0o755);
        let link = root.with_extension("link");
        std::os::unix::fs::symlink(&root, &link).unwrap();
        assert!(private_directory(&link, true).is_err());
        std::fs::remove_file(link).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[tokio::test]
    async fn stale_socket_is_not_running_and_next_owner_reclaims_it() {
        let fixture = super::super::tests::Fixture::new();
        let key = super::super::endpoint_key(&fixture.registry).unwrap();
        let path = endpoint(&key, true).unwrap();
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        drop(listener);
        assert_eq!(
            connect(&key).await.unwrap_err().code(),
            "runtime-not-running"
        );
        let listener = Listener::bind(&key).unwrap();
        assert!(Listener::bind(&key).is_err());
        drop(listener);
        assert!(!path.exists());
    }
}
