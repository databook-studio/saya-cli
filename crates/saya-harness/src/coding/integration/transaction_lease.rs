//! One process-shared writer for an original tree across apply and recovery.
//! Lock files are never removed: all contenders must lock the same inode.

use super::{ApplyError, transaction::ApplyBackError};
use crate::workspace::Workspace;

/// A child forked by another thread keeps an inherited copy of a dropped lease
/// descriptor until it execs, so the kernel can release the lock a few
/// milliseconds after its owner finished. A contender waits this long, no
/// longer, before reporting the tree busy.
#[cfg(unix)]
const LEASE_HANDOFF_WAIT: std::time::Duration = std::time::Duration::from_millis(250);
#[cfg(unix)]
const LEASE_POLL: std::time::Duration = std::time::Duration::from_millis(2);

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
    let deadline = std::time::Instant::now() + LEASE_HANDOFF_WAIT;
    loop {
        // SAFETY: flock receives a live owned descriptor and valid operation flags.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            break;
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::WouldBlock {
            return Err(ApplyError::Fs.into());
        }
        if std::time::Instant::now() >= deadline {
            return Err(ApplyBackError::OperationBusy);
        }
        std::thread::sleep(LEASE_POLL);
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
