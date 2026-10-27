use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

/// An OS-held lock on the registry file itself, not a pathname-derived sidecar.
/// Aliases and hard links therefore contend for the same physical resource.
pub(crate) struct Ownership {
    _file: File,
    pub path: PathBuf,
}

impl Ownership {
    pub fn acquire(path: &Path) -> anyhow::Result<Self> {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        lock(&file).map_err(lock_error)?;
        let path = std::fs::canonicalize(path)?;
        Ok(Self { _file: file, path })
    }
}

fn lock_error(error: std::io::Error) -> anyhow::Error {
    #[cfg(target_os = "macos")]
    if !matches!(error.raw_os_error(), Some(libc::EAGAIN | libc::EACCES)) {
        return error.into();
    }
    anyhow::Error::new(error).context("registry-already-owned")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn lock(file: &File) -> std::io::Result<()> {
    fs2::FileExt::try_lock_exclusive(file)
}

#[cfg(target_os = "macos")]
fn lock(file: &File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    // Darwin's whole-file flock conflicts with SQLite's POSIX locks. An OFD
    // byte outside SQLite's range retains last-close lifetime and exclusion
    // across separate opens, even in this process, without extending the file.
    let lock = libc::flock {
        l_start: i64::MAX - 1,
        l_len: 1,
        l_pid: 0,
        l_type: libc::F_WRLCK,
        l_whence: libc::SEEK_SET as libc::c_short,
    };
    let result = unsafe {
        libc::fcntl(
            file.as_raw_fd(),
            libc::F_OFD_SETLK,
            &lock as *const libc::flock,
        )
    };
    if result == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Metadata-only endpoint identity. This neither opens SQLite nor takes an
/// ownership lock, and path aliases and hard links identify the same file.
pub(crate) fn physical_identity(path: &Path) -> std::io::Result<String> {
    let canonical = std::fs::canonicalize(path)?;
    physical_identity_canonical(&canonical)
}

#[cfg(unix)]
fn physical_identity_canonical(path: &Path) -> std::io::Result<String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "registry is not a regular file",
        ));
    }
    Ok(format!("unix:{}:{}", metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn physical_identity_canonical(path: &Path) -> std::io::Result<String> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    // Like the store identity helper, query the open handle, not path text.
    // Read attributes suffices even while the owner holds its registry lock.
    let file = OpenOptions::new()
        .read(true)
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "registry is not a regular file",
        ));
    }
    let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle() as isize,
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let id: String = info
        .FileId
        .Identifier
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(format!("windows:{}:{id}", info.VolumeSerialNumber))
}

#[cfg(windows)]
fn lock(file: &File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
    };
    use windows_sys::Win32::System::IO::OVERLAPPED;
    // Outside SQLite's locking bytes, without extending the file. The kernel
    // releases this byte-range lock on process death, including PID reuse.
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    overlapped.Anonymous.Anonymous.Offset = 0xffff_fffe;
    overlapped.Anonymous.Anonymous.OffsetHigh = 0x7fff_ffff;
    let ok = unsafe {
        LockFileEx(
            file.as_raw_handle() as isize,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &mut overlapped,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn only_lock_conflicts_are_classified_as_already_owned() {
        for code in [
            libc::EAGAIN,
            libc::EACCES,
            libc::EINVAL,
            libc::ENOTSUP,
            libc::EIO,
        ] {
            let error = super::lock_error(std::io::Error::from_raw_os_error(code));
            assert_eq!(
                error
                    .downcast_ref::<std::io::Error>()
                    .unwrap()
                    .raw_os_error(),
                Some(code)
            );
            assert_eq!(
                error.to_string() == "registry-already-owned",
                matches!(code, libc::EAGAIN | libc::EACCES)
            );
        }
    }
}
