//! Bounded private continuation artifacts, stored as data without grants.
use super::CodingError;
#[cfg(windows)]
use std::io::{Seek as _, SeekFrom};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
pub(super) fn write_new(path: &Path, bytes: &[u8], maximum: usize) -> Result<(), CodingError> {
    if bytes.len() > maximum {
        return Err(CodingError::InvalidRequest);
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    windows_flags(&mut options);
    let mut file = options.open(path).map_err(CodingError::owner)?;
    refuse_reparse(&file)?;
    file.write_all(bytes).map_err(CodingError::owner)?;
    file.sync_all().map_err(CodingError::owner)
}
/// Replaces a bounded private artifact in place. A resume finalizes again, so
/// the outcome must be overwritable; the file stays private and symlink-safe.
pub(super) fn write_replace(path: &Path, bytes: &[u8], maximum: usize) -> Result<(), CodingError> {
    if bytes.len() > maximum {
        return Err(CodingError::InvalidRequest);
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    windows_flags(&mut options);
    let mut file = options.open(path).map_err(CodingError::owner)?;
    refuse_reparse(&file)?;
    file.write_all(bytes).map_err(CodingError::owner)?;
    file.sync_all().map_err(CodingError::owner)
}

pub(super) fn read(path: &Path, maximum: usize) -> Result<Vec<u8>, CodingError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    windows_flags(&mut options);
    let file = options.open(path).map_err(CodingError::owner)?;
    refuse_reparse(&file)?;
    if !file.metadata().map_err(CodingError::owner)?.is_file() {
        return Err(CodingError::StateDirectory);
    }
    let mut bytes = Vec::new();
    file.take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(CodingError::owner)?;
    if bytes.len() > maximum {
        return Err(CodingError::InvalidRequest);
    }
    Ok(bytes)
}
// Bounded local artifact appends are infrequent; one process lock prevents
// interleaved metadata/length checks without introducing a second job registry.
static APPEND_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub(super) fn append_line(path: &Path, bytes: &[u8], maximum: usize) -> Result<(), CodingError> {
    let _guard = APPEND_LOCK
        .lock()
        .map_err(|_| CodingError::StateDirectory)?;
    let mut file = open_append_handle(path)?;
    // Nonblocking OS lock keeps cap checks atomic across cooperating processes.
    // Contention is an explicit owner failure, never an unbounded wait.
    file.try_lock().map_err(CodingError::owner)?;
    let metadata = file.metadata().map_err(CodingError::owner)?;
    if !metadata.is_file() || metadata.len().saturating_add(bytes.len() as u64 + 1) > maximum as u64
    {
        return Err(CodingError::InvalidRequest);
    }
    #[cfg(windows)]
    file.seek(SeekFrom::End(0)).map_err(CodingError::owner)?;
    file.write_all(bytes).map_err(CodingError::owner)?;
    file.write_all(b"\n").map_err(CodingError::owner)?;
    file.sync_all().map_err(CodingError::owner)
}

/// Opens a job artifact for durable appending. Windows append handles
/// (`FILE_APPEND_DATA`) cannot be flushed — `FlushFileBuffers` needs
/// `FILE_WRITE_DATA` — so there the handle asks for ordinary write access and
/// every writer seeks to the end first; unix keeps its append flag, which is
/// atomic at the kernel.
fn open_append_handle(path: &Path) -> Result<File, CodingError> {
    let mut options = OpenOptions::new();
    #[cfg(windows)]
    options.write(true).create(true);
    #[cfg(not(windows))]
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    windows_flags(&mut options);
    let file = options.open(path).map_err(CodingError::owner)?;
    refuse_reparse(&file)?;
    Ok(file)
}

/// Asks Windows for the link itself, so a reparse point planted at a job
/// artifact's path is refused instead of followed. Unix sets `O_NOFOLLOW` in
/// each call site's own flag block.
fn windows_flags(options: &mut OpenOptions) {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    #[cfg(not(windows))]
    let _ = options;
}

/// Refuses an opened handle that names a reparse point.
fn refuse_reparse(file: &std::fs::File) -> Result<(), CodingError> {
    #[cfg(windows)]
    if saya_harness::workspace::windows_identity::is_reparse_point(file)
        .map_err(CodingError::owner)?
    {
        return Err(CodingError::StateDirectory);
    }
    #[cfg(not(windows))]
    let _ = file;
    Ok(())
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    /// Real Windows refuses `FlushFileBuffers` on an append-only handle
    /// (`FILE_APPEND_DATA` without `FILE_WRITE_DATA`). An append handle also
    /// ignores the seek position, so a write at offset zero proves the
    /// returned handle carries ordinary write access.
    #[test]
    fn append_handle_flushes_and_writes_at_the_seek_position() {
        let path = std::env::temp_dir().join(format!(
            "saya-context-append-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut file = open_append_handle(&path).unwrap();
        file.seek(SeekFrom::End(0)).unwrap();
        file.write_all(b"first").unwrap();
        file.sync_all().unwrap();
        file.seek(SeekFrom::End(0)).unwrap();
        file.write_all(b"second").unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert_eq!(std::fs::read(&path).unwrap(), b"firstsecond");
        let mut file = open_append_handle(&path).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(b"X").unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert_eq!(std::fs::read(&path).unwrap(), b"Xirstsecond");
        let _ = std::fs::remove_file(&path);
    }
}
