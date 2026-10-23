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
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        lock(&file).context("registry-already-owned")?;
        let path = std::fs::canonicalize(path)?;
        Ok(Self { _file: file, path })
    }
}

#[cfg(unix)]
fn lock(file: &File) -> std::io::Result<()> {
    fs2::FileExt::try_lock_exclusive(file)
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
