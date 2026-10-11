//! One process-shared writer for an original tree across apply and recovery.
//! Lock files are never removed: all contenders must lock the same inode.

use super::{ApplyError, transaction::ApplyBackError};
use crate::workspace::Workspace;

pub(super) struct ApplyBackLease {
    _file: std::fs::File,
}

#[cfg(unix)]
pub(super) fn acquire(
    state_dir: &std::path::Path,
    original: &Workspace,
) -> Result<ApplyBackLease, ApplyBackError> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let [device, inode] = super::recover_binding::root_identity(original)?;
    let dir = state_dir.join("coding/apply-locks");
    std::fs::create_dir_all(&dir).map_err(|_| ApplyError::Fs)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(dir.join(format!("{device:x}-{inode:x}.lock")))
        .map_err(|_| ApplyError::Fs)?;
    if !file.metadata().map_err(|_| ApplyError::Fs)?.is_file() {
        return Err(ApplyError::Fs.into());
    }
    // SAFETY: flock receives a live owned descriptor and valid operation flags.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock {
                ApplyBackError::OperationBusy
            } else {
                ApplyError::Fs.into()
            },
        );
    }
    Ok(ApplyBackLease { _file: file })
}

#[cfg(not(unix))]
pub(super) fn acquire(
    _state_dir: &std::path::Path,
    _original: &Workspace,
) -> Result<ApplyBackLease, ApplyBackError> {
    Err(ApplyError::Fs.into())
}
