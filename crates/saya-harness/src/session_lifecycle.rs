//! Session-lifecycle ownership for the harness: the two operations that
//! must never race a session's owner. [`acquire_session_lock`] hands out
//! the per-session single-writer lock only while the id carries no
//! deletion tombstone, re-checking the tombstone after the lock is held so
//! a deletion that raced the claim is still caught.
//! [`delete_session`] refuses a live session, then holds the lock while
//! the store's tombstone is made durable and its cleanup runs, so
//! acquisition and deletion exclude each other through the same lock file
//! the owner holds.
//!
//! Exclusivity after cleanup relies on the tombstone being checked before
//! and after lock acquisition, never on the removed lock file; no lock
//! inode is removed while it is the thing excluding a concurrent deleter
//! (the deleter itself holds it until the tombstone is durable). The
//! store's tombstone lives outside the removable `<sessions>` tree, so it
//! survives every partial cleanup and a retry converges.

use std::{fs, path::Path};

use saya_store::{DeleteStatus, FsSessionStore, StoreError};

use crate::{HarnessError, io_error, lock::RunLock};

/// A held per-session lock. Dropping it releases best effort: the lock
/// file is removed only while it still names this process, and a file
/// already removed with the session state dir is fine.
#[derive(Debug)]
pub struct SessionLock {
    lock: RunLock,
}

impl SessionLock {
    /// The pid the lock names: this process's, like [`RunLock::pid`].
    pub fn pid(&self) -> u32 {
        self.lock.pid()
    }
}

/// Refuses when `id` carries a deletion tombstone. The pre-creation check
/// for callers about to create `<sessions>/<id>/`, and the first check of
/// [`acquire_session_lock`]: a deleted or being-deleted id never gets as
/// far as a lock. Invalid ids are refused through [`HarnessError::Store`].
pub fn ensure_not_deleted(store: &FsSessionStore, id: &str) -> Result<(), HarnessError> {
    match store.is_deleted(id) {
        Ok(true) => Err(HarnessError::SessionDeleted { id: id.to_owned() }),
        Ok(false) => Ok(()),
        Err(source) => Err(HarnessError::Store { source }),
    }
}

/// Acquires the per-session lock for `id` at `<state_dir>/lock`; the
/// caller creates `state_dir`. The tombstone is checked before the lock
/// and again once it is held: a session deleted while the lock was being
/// claimed is refused — releasing the lock and removing the now-empty
/// state dir best effort — so a deleted id can never be acquired, and a
/// tombstone written by a party that did not honour the lock cannot be
/// adopted by holding it.
pub fn acquire_session_lock(
    store: &FsSessionStore,
    state_dir: &Path,
    id: &str,
) -> Result<SessionLock, HarnessError> {
    ensure_not_deleted(store, id)?;
    let lock = RunLock::acquire(state_dir.join("lock"))?;
    match store.is_deleted(id) {
        Ok(false) => Ok(SessionLock { lock }),
        Ok(true) => {
            drop(lock);
            let _ = fs::remove_dir(state_dir);
            Err(HarnessError::SessionDeleted { id: id.to_owned() })
        }
        Err(source) => {
            drop(lock);
            Err(HarnessError::Store { source })
        }
    }
}

/// Deletes session `id` under its own lock. While a live process holds
/// `<sessions>/<id>/lock`, nothing happens: `LockHeld` is returned, the
/// session's data untouched. Otherwise the lock is held, the tombstone is
/// made durable through the store BEFORE any removal, the store's cleanup
/// runs, and its [`DeleteStatus`] is reported. A tombstone left by a
/// crashed attempt is retried, not refused: `begin_delete` is idempotent
/// and `finish_delete` converges.
pub fn delete_session(
    store: &FsSessionStore,
    sessions_root: &Path,
    id: &str,
) -> Result<DeleteStatus, HarnessError> {
    // Validates the id through the store before anything is created. The
    // value is deliberately unused: a pending tombstone must retry.
    store.is_deleted(id).map_err(store_error)?;
    let state_dir = sessions_root.join(id);
    let mut created = false;
    if !state_dir.is_dir() {
        fs::create_dir_all(&state_dir)
            .map_err(|error| io_error("create session state dir", &state_dir, error))?;
        created = true;
        #[cfg(unix)]
        if let Err(error) = set_private_mode(&state_dir) {
            let _ = fs::remove_dir(&state_dir);
            return Err(error);
        }
    }
    let lock = match RunLock::acquire(state_dir.join("lock")) {
        Ok(lock) => lock,
        Err(error) => {
            if created {
                // Only an empty dir is removed; a live holder's lock file
                // keeps it in place.
                let _ = fs::remove_dir(&state_dir);
            }
            return Err(error);
        }
    };
    let outcome = store
        .begin_delete(id)
        .map_err(store_error)
        .and_then(|()| store.finish_delete(id).map_err(store_error));
    drop(lock);
    outcome
}

#[cfg(unix)]
fn set_private_mode(state_dir: &Path) -> Result<(), HarnessError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(state_dir, fs::Permissions::from_mode(0o700))
        .map_err(|error| io_error("restrict session state dir", state_dir, error))
}

fn store_error(source: StoreError) -> HarnessError {
    HarnessError::Store { source }
}
