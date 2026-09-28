//! The cross-process mutation lock for the investigation repository (A1):
//! every mutation — create, update, delete, and binding writes — runs
//! entirely under `<root>/.lock` as one critical section; reads never take
//! the lock. The lock is an OS advisory exclusive file lock (`File::try_lock`:
//! `flock` on unix, `LockFileEx` on Windows) taken on a lock file created if
//! missing (0600 on unix) and never truncated. Contention retries for a
//! bounded budget, then reports `Unavailable`; a lock that cannot be probed
//! at all is also a refusal, never an unlocked proceed. The guard holds the
//! file, so dropping it unlocks (and closes), and the OS releases the lock
//! when the holder's process dies however abruptly. There is no staleness,
//! age, pid, or inode protocol, and lock code never deletes or renames the
//! lock file.

use crate::StoreError;
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// How long a contended acquisition retries before reporting `Unavailable`.
pub(crate) const LOCK_WAIT: Duration = Duration::from_secs(2);

const FIRST_SLEEP: Duration = Duration::from_millis(2);
const MAX_SLEEP: Duration = Duration::from_millis(50);

/// An exclusively held repository lock: holding the file is holding the OS
/// advisory lock, and the file stays open for the guard's whole life.
#[derive(Debug)]
pub(crate) struct RepositoryLock {
    file: File,
}

impl Drop for RepositoryLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Acquires the repository lock at `<root>/.lock`: opens the lock file
/// (creating it if missing, never truncating it), then `try_lock`s it in a
/// bounded retry loop. Reports `Unavailable` when the budget is exhausted
/// and whenever the lock cannot be probed at all — it never proceeds
/// unlocked.
pub(crate) fn acquire(root: &Path, wait: Duration) -> Result<RepositoryLock, StoreError> {
    let deadline = Instant::now() + wait;
    let file = open_lock_file(&root.join(".lock"))?;
    let mut sleep = FIRST_SLEEP;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(RepositoryLock { file }),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(_)) => return Err(StoreError::unavailable()),
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(StoreError::unavailable());
        }
        std::thread::sleep(sleep.min(remaining));
        sleep = (sleep * 2).min(MAX_SLEEP);
    }
}

/// Opens the lock file the OS advisory lock is taken on: created if missing
/// (0600 on unix), never truncated — the file's contents are nobody's
/// business, so opening never destroys what another reader may still see.
fn open_lock_file(path: &PathBuf) -> Result<File, StoreError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|_| StoreError::unavailable())
}

impl super::InvestigationRepository {
    /// Acquires the mutation lock after ensuring the root exists: every
    /// mutation holds it for its whole read/check/publish critical
    /// section.
    pub(crate) fn lock_for_mutation(&self) -> Result<RepositoryLock, StoreError> {
        self.ensure_root()?;
        acquire(&self.root, self.lock_wait)
    }

    /// The production construction with a test-sized contention budget: how
    /// long contended mutations retry.
    #[cfg(test)]
    pub(crate) fn with_lock_params(root: PathBuf, lock_wait: Duration) -> Self {
        Self { root, lock_wait }
    }
}
