//! Bounded private continuation artifacts, stored as data without grants.
use super::CodingError;
use std::{
    fs::OpenOptions,
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
    let mut file = options.open(path).map_err(CodingError::owner)?;
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
    let file = options.open(path).map_err(CodingError::owner)?;
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
    let mut options = OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(path).map_err(CodingError::owner)?;
    let metadata = file.metadata().map_err(CodingError::owner)?;
    if !metadata.is_file() || metadata.len().saturating_add(bytes.len() as u64 + 1) > maximum as u64
    {
        return Err(CodingError::InvalidRequest);
    }
    file.write_all(bytes).map_err(CodingError::owner)?;
    file.write_all(b"\n").map_err(CodingError::owner)?;
    file.sync_all().map_err(CodingError::owner)
}
