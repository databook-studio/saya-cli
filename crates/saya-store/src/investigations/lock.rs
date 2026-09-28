//! The cross-process mutation lock for the investigation repository (A1):
//! every mutation — create, update, delete, and binding writes — runs
//! entirely under `<root>/.lock` as one critical section; reads never take
//! the lock. The lock is a `create_new` file (0600) holding the holder's
//! pid and acquisition time. Contention retries for a bounded budget, then
//! reports `Unavailable`; breaking a stale lock is `stale`'s concern.
//! Release removes the file only when its contents still match what this
//! guard wrote, so a guard whose lock was stale-broken cannot delete the
//! next writer's fresh lock.

use super::stale::{break_if_stale, now_unix_ms};
use crate::StoreError;
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// How long a contended acquisition retries before reporting `Unavailable`.
pub(crate) const LOCK_WAIT: Duration = Duration::from_secs(2);
/// A lock held longer than this is treated as abandoned and may be broken.
pub(crate) const STALE_AFTER: Duration = Duration::from_secs(30);

const FIRST_SLEEP: Duration = Duration::from_millis(2);
const MAX_SLEEP: Duration = Duration::from_millis(50);

/// The on-disk lock contents: the holder's pid and acquisition time.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LockContents {
    pub(super) pid: u32,
    pub(super) acquired_unix_ms: i64,
}

/// An exclusively held repository lock. Dropping it releases the lock file
/// if it is still exactly the one this guard wrote.
pub(crate) struct RepositoryLock {
    path: PathBuf,
    contents: LockContents,
}

impl Drop for RepositoryLock {
    fn drop(&mut self) {
        let owned = std::fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<LockContents>(&bytes).ok())
            .is_some_and(|current| current == self.contents);
        if owned {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Acquires the repository lock at `<root>/.lock`: retries under contention
/// for `wait`, breaks a verified-stale lock once, and reports `Unavailable`
/// when the budget is exhausted.
pub(crate) fn acquire(
    root: &Path,
    wait: Duration,
    stale_after: Duration,
) -> Result<RepositoryLock, StoreError> {
    let path = root.join(".lock");
    let deadline = Instant::now() + wait;
    let mut sleep = FIRST_SLEEP;
    let mut broke_stale = false;
    loop {
        match claim(&path) {
            Ok(contents) => return Ok(RepositoryLock { path, contents }),
            Err(ClaimError::Failed) => return Err(StoreError::unavailable()),
            Err(ClaimError::Exists) => {}
        }
        if !broke_stale && break_if_stale(&path, stale_after) {
            broke_stale = true;
            continue;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(StoreError::unavailable());
        }
        std::thread::sleep(sleep.min(remaining));
        sleep = (sleep * 2).min(MAX_SLEEP);
    }
}

enum ClaimError {
    Exists,
    Failed,
}

/// Writes our pid and acquisition time into a freshly created `.lock`.
/// `create_new` makes the claim exclusive and the contents are fsynced
/// before the claim is reported; a failed write removes the file we just
/// created, never one another holder owns.
fn claim(path: &Path) -> Result<LockContents, ClaimError> {
    let contents = LockContents {
        pid: std::process::id(),
        acquired_unix_ms: now_unix_ms(),
    };
    let payload = match serde_json::to_vec(&contents) {
        Ok(payload) => payload,
        Err(_) => return Err(ClaimError::Failed),
    };
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::AlreadyExists => ClaimError::Exists,
        _ => ClaimError::Failed,
    })?;
    if file
        .write_all(&payload)
        .and_then(|()| file.sync_all())
        .is_err()
    {
        let _ = std::fs::remove_file(path);
        return Err(ClaimError::Failed);
    }
    Ok(contents)
}

impl super::InvestigationRepository {
    /// Acquires the mutation lock after ensuring the root exists: every
    /// mutation holds it for its whole read/check/publish critical
    /// section.
    pub(crate) fn lock_for_mutation(&self) -> Result<RepositoryLock, StoreError> {
        self.ensure_root()?;
        acquire(&self.root, self.lock_wait, self.stale_after)
    }

    /// The production construction with test-sized lock budgets: how long
    /// contended mutations retry, and when a lock counts as stale.
    #[cfg(test)]
    pub(crate) fn with_lock_params(
        root: PathBuf,
        lock_wait: Duration,
        stale_after: Duration,
    ) -> Self {
        Self {
            root,
            lock_wait,
            stale_after,
        }
    }
}
