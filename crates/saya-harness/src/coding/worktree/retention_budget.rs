//! Disk-pressure admission refuses new work; it never evicts retained user work.
use super::CleanupError;
use crate::coding::git::GitService;
use saya_agent::CancellationToken;
use saya_store::SqliteStateStore;
use saya_types::{JobId, ManagedCheckoutLifecycle};
use std::{
    path::Path,
    time::{Duration, Instant},
};

/// Check actual owned-checkout bytes before allocating another checkout. The
/// requested reserve is caller-estimated snapshot materialization space. This
/// counts the current job's checkouts only; host-wide quota remains host policy.
pub async fn ensure_retention_capacity(
    store: &SqliteStateStore,
    service: &GitService,
    job: &JobId,
    requested_bytes: u64,
    maximum_bytes: u64,
    cancellation: &CancellationToken,
) -> Result<u64, CleanupError> {
    if maximum_bytes == 0
        || maximum_bytes > 16 * 1024 * 1024 * 1024
        || requested_bytes > maximum_bytes
    {
        return Err(CleanupError::RetentionBudgetExceeded);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut cursor = None;
    let mut bytes = requested_bytes;
    let mut entries = 0;
    let mut checkouts = 0;
    loop {
        let page = store
            .list_managed_checkouts(job, cursor.as_ref(), 100)
            .await?;
        for record in page.items {
            checkouts += 1;
            if checkouts > 256 {
                return Err(CleanupError::RetentionBudgetExceeded);
            }
            if record.current_lifecycle == ManagedCheckoutLifecycle::Removed {
                continue;
            }
            let id = record.ownership.managed_checkout_id();
            if record.ownership.managed_path() != &service.managed_path(id)? {
                return Err(super::OwnershipError::PathMismatch.into());
            }
            let path = Path::new(record.ownership.managed_path().as_str());
            bytes = scan(
                path,
                bytes,
                maximum_bytes,
                &mut entries,
                deadline,
                cancellation,
            )?;
        }
        let Some(next) = page.next_cursor else {
            return Ok(bytes - requested_bytes);
        };
        cursor = Some(next);
    }
}

#[cfg(not(unix))]
fn scan(
    _path: &Path,
    _bytes: u64,
    _maximum: u64,
    _entries: &mut usize,
    _deadline: Instant,
    _cancel: &CancellationToken,
) -> Result<u64, CleanupError> {
    Err(CleanupError::RequireOperator)
}
#[cfg(unix)]
fn scan(
    path: &Path,
    bytes: u64,
    maximum: u64,
    entries: &mut usize,
    deadline: Instant,
    cancel: &CancellationToken,
) -> Result<u64, CleanupError> {
    use crate::workspace::fd::{DirStream, open_root_fd};
    use std::{ffi::OsStr, os::fd::AsRawFd};
    let root = open_root_fd(path.as_os_str()).map_err(|_| CleanupError::RequireOperator)?;
    let stream = DirStream::open_at(root.as_raw_fd(), OsStr::new("."))
        .map_err(|_| CleanupError::RequireOperator)?;
    let mut stack = vec![stream];
    let mut bytes = bytes;
    while !stack.is_empty() {
        if cancel.is_cancelled() || Instant::now() >= deadline || stack.len() > 256 {
            return Err(CleanupError::RetentionBudgetExceeded);
        }
        let directory = stack.last_mut().ok_or(CleanupError::RequireOperator)?;
        let Some((name, _)) = directory
            .next()
            .map_err(|_| CleanupError::RequireOperator)?
        else {
            stack.pop();
            continue;
        };
        *entries += 1;
        if *entries > 100_000 {
            return Err(CleanupError::RetentionBudgetExceeded);
        }
        let stat = crate::workspace::fd::stat_at(directory.dir_fd(), &name)
            .map_err(|_| CleanupError::RequireOperator)?;
        if stat.is_dir() {
            let child = DirStream::open_at(directory.dir_fd(), &name)
                .map_err(|_| CleanupError::RequireOperator)?;
            stack.push(child);
        } else if stat.is_file() {
            bytes = bytes
                .checked_add(stat.len())
                .ok_or(CleanupError::RetentionBudgetExceeded)?;
            if bytes > maximum {
                return Err(CleanupError::RetentionBudgetExceeded);
            }
        } else {
            return Err(CleanupError::RequireOperator);
        }
    }
    Ok(bytes)
}
