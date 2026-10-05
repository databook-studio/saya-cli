//! The removal half of two-phase session deletion. With a tombstone present
//! (see [`super::tombstone`]): remove the session JSON, the per-session
//! state directory, and the staged temps and quarantined copies of exactly
//! that id — never following a symlink — then the tombstone last. Anything
//! not removable reports [`DeleteStatus::Pending`] with the tombstone kept,
//! so a retry converges; data outside the id's own paths is never touched.
//! The caller holds the exclusive lock and has refused a live session (D12).

use super::tombstone;
use crate::{StoreError, filesystem::FsSessionStore, private_file::io_error};
use std::{fs, io::ErrorKind, path::Path};

/// The outcome of [`FsSessionStore::finish_delete`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteStatus {
    /// Every removable path is gone, including the tombstone.
    Complete,
    /// Some paths could not be removed; the tombstone is kept and a retry
    /// converges (`remaining` counts the paths still present).
    Pending { remaining: usize },
}

impl FsSessionStore {
    /// Marks `id` for deletion by writing the durable tombstone outside the
    /// sessions tree: the session is invisible and saves of it are refused
    /// from this moment; nothing is removed yet. Idempotent.
    ///
    /// The CALLER must hold the session's exclusive lock and have refused a
    /// live session first (packet D12); this layer is persistence mechanics
    /// only.
    pub fn begin_delete(&self, id: &str) -> Result<(), StoreError> {
        self.path(id)?;
        tombstone::write(self.sessions_root(), id)
    }

    /// Whether a tombstone exists for `id` — the session is hidden.
    pub fn is_deleted(&self, id: &str) -> Result<bool, StoreError> {
        self.path(id)?;
        tombstone::exists(self.sessions_root(), id)
    }

    /// Removes what the tombstone covers: the session JSON, the per-session
    /// state directory, staged temps and quarantined copies of this id,
    /// then the tombstone itself. Without a tombstone this removes NOTHING
    /// and reports [`DeleteStatus::Complete`] — a bare id never deletes.
    pub fn finish_delete(&self, id: &str) -> Result<DeleteStatus, StoreError> {
        let session_path = self.path(id)?;
        if !tombstone::exists(self.sessions_root(), id)? {
            return Ok(DeleteStatus::Complete);
        }
        let mut remaining = 0usize;
        if remove_path_no_follow(&session_path).is_err() {
            remaining += 1;
        }
        // Derived only after `path` validated the id above.
        if remove_path_no_follow(&self.sessions_root().join(id)).is_err() {
            remaining += 1;
        }
        remaining += sweep_id_owned_names(self.sessions_root(), id)?;
        if remaining > 0 {
            return Ok(DeleteStatus::Pending { remaining });
        }
        if remove_path_no_follow(&tombstone::path(self.sessions_root(), id)?).is_err() {
            return Ok(DeleteStatus::Pending { remaining: 1 });
        }
        Ok(DeleteStatus::Complete)
    }

    /// The ids with a tombstone, sorted — the pending deletions.
    pub fn pending_deletions(&self) -> Result<Vec<String>, StoreError> {
        tombstone::deleted_ids(self.sessions_root())
    }
}

/// Removes `path` without ever following a symlink: a link — at the path
/// itself or found while emptying a directory — is UNLINKED as a name, its
/// target untouched. A directory is emptied child by child, each child
/// classified again, then removed; a file is unlinked; absent counts as
/// removed. [`std::fs::remove_dir_all`] is deliberately not used: a symlink
/// planted at the final component must never select a target for removal.
pub(crate) fn remove_path_no_follow(path: &Path) -> Result<(), StoreError> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error(error)),
    };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return fs::remove_file(path).map_err(io_error);
    }
    let entries = fs::read_dir(path).map_err(io_error)?;
    let mut blocked = false;
    for entry in entries {
        match entry {
            Ok(entry) => blocked |= remove_path_no_follow(&entry.path()).is_err(),
            Err(_) => blocked = true,
        }
    }
    if blocked {
        return Err(StoreError::unavailable());
    }
    fs::remove_dir(path).map_err(io_error)
}

/// Removes the staged temps (`.{id}.json.<pid>.<seq>.tmp`) and quarantined
/// copies (`<id>.corrupt-<stamp>`) of exactly `id`, scanning the sessions
/// root once. Matching is strict so `a-2`'s files and non-numeric stamps
/// (`a.corrupt-x`) are never touched; returns how many names remain.
fn sweep_id_owned_names(root: &Path, id: &str) -> Result<usize, StoreError> {
    let entries = match fs::read_dir(root) {
        Ok(value) => value,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
        Err(_) => return Err(StoreError::unavailable()),
    };
    let mut remaining = 0usize;
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if is_staged_temp_name(name, id) || is_quarantine_name(name, id) {
            remaining += usize::from(remove_path_no_follow(&entry.path()).is_err());
        }
    }
    Ok(remaining)
}

/// Exactly `.{id}.json.<digits>.<digits>.tmp`, as `temporary_path` mints.
fn is_staged_temp_name(name: &str, id: &str) -> bool {
    let Some(rest) = name
        .strip_prefix('.')
        .and_then(|rest| rest.strip_prefix(id))
        .and_then(|rest| rest.strip_prefix(".json."))
    else {
        return false;
    };
    let mut parts = rest.split('.');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(pid), Some(sequence), Some("tmp"), None) => is_digits(pid) && is_digits(sequence),
        _ => false,
    }
}

/// Exactly `<id>.corrupt-<digits>`, as `quarantine` mints.
fn is_quarantine_name(name: &str, id: &str) -> bool {
    name.strip_prefix(id)
        .and_then(|rest| rest.strip_prefix(".corrupt-"))
        .is_some_and(is_digits)
}

fn is_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}
