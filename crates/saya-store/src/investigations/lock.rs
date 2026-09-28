//! The cross-process mutation lock for the investigation repository (A1):
//! every mutation — create, update, delete, and binding writes — runs
//! entirely under `<root>/.lock` as one critical section; reads never take
//! the lock. The lock is a `create_new` file (0600) holding the holder's
//! pid and acquisition time. Contention retries for a bounded budget, then
//! reports `Unavailable`; breaking a stale lock is `stale`'s concern.
//! Release removes the file only when its contents still match what this
//! guard wrote AND — on unix — the file at the path is still the very one
//! this guard claimed (same device and inode), so a guard whose lock was
//! stale-broken cannot delete the next writer's fresh claim even when the
//! successor's contents are byte-identical (same pid, same millisecond).
//! On platforms without file identity, release falls back to the contents
//! check alone.

use super::stale::{break_if_stale, now_unix_ms};
use crate::StoreError;
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

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

/// The filesystem identity of the lock file this guard claimed: release
/// removes the file only while the path still resolves to this device and
/// inode, so a successor's fresh claim at the same path — even with
/// byte-identical contents — is never deleted.
#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
}

#[cfg(unix)]
impl FileIdentity {
    /// The identity of an open claim handle; `None` when it cannot be
    /// read, in which case release falls back to the contents check alone.
    fn of(file: &std::fs::File) -> Option<Self> {
        let metadata = file.metadata().ok()?;
        Some(Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
        })
    }

    /// Whether the file now at `path` is still the one this identity was
    /// taken from.
    fn still_at(&self, path: &Path) -> bool {
        std::fs::metadata(path)
            .ok()
            .is_some_and(|metadata| metadata.dev() == self.dev && metadata.ino() == self.ino)
    }
}

/// An exclusively held repository lock. Dropping it releases the lock file
/// if it is still exactly the one this guard wrote — same contents, and on
/// unix the same file identity.
pub(crate) struct RepositoryLock {
    path: PathBuf,
    contents: LockContents,
    #[cfg(unix)]
    identity: Option<FileIdentity>,
}

impl Drop for RepositoryLock {
    fn drop(&mut self) {
        let contents_match = std::fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<LockContents>(&bytes).ok())
            .is_some_and(|current| current == self.contents);
        #[cfg(unix)]
        let identity_match = self
            .identity
            .as_ref()
            .is_none_or(|identity| identity.still_at(&self.path));
        #[cfg(not(unix))]
        let identity_match = true;
        if contents_match && identity_match {
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
    let mut aside: Option<PathBuf> = None;
    loop {
        match claim(&path) {
            Ok(lock) => {
                // The stale break's aside file is dead weight once this
                // acquisition re-claims; remove it best-effort (R4-5).
                if let Some(aside) = aside.take() {
                    let _ = std::fs::remove_file(&aside);
                }
                return Ok(lock);
            }
            Err(ClaimError::Failed) => return Err(StoreError::unavailable()),
            Err(ClaimError::Exists) => {}
        }
        if !broke_stale && let Some(broken_aside) = break_if_stale(&path, stale_after) {
            broke_stale = true;
            aside = Some(broken_aside);
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

/// Writes our pid and acquisition time into a freshly created `.lock` and
/// returns the guard for it. `create_new` makes the claim exclusive and
/// the contents are fsynced before the claim is reported; a failed write
/// removes the file we just created, never one another holder owns.
fn claim(path: &Path) -> Result<RepositoryLock, ClaimError> {
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
    #[cfg(unix)]
    let identity = FileIdentity::of(&file);
    if file
        .write_all(&payload)
        .and_then(|()| file.sync_all())
        .is_err()
    {
        let _ = std::fs::remove_file(path);
        return Err(ClaimError::Failed);
    }
    Ok(RepositoryLock {
        path: path.to_path_buf(),
        contents,
        #[cfg(unix)]
        identity,
    })
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
