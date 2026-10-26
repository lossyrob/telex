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
        use anyhow::Context;
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        #[cfg(test)]
        let pilot = std::time::Instant::now();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        #[cfg(test)]
        crate::pilot209::record("ownership:create-dir-all", pilot);
        #[cfg(test)]
        let pilot = std::time::Instant::now();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        #[cfg(test)]
        crate::pilot209::record("ownership:file-open", pilot);
        #[cfg(test)]
        let pilot = std::time::Instant::now();
        lock(&file).context("registry-already-owned")?;
        #[cfg(test)]
        crate::pilot209::record("ownership:lock", pilot);
        #[cfg(test)]
        let pilot = std::time::Instant::now();
        let path = std::fs::canonicalize(path)?;
        #[cfg(test)]
        crate::pilot209::record("ownership:canonicalize", pilot);
        Ok(Self { _file: file, path })
    }
}

#[cfg(unix)]
fn lock(file: &File) -> std::io::Result<()> {
    fs2::FileExt::try_lock_exclusive(file)
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
