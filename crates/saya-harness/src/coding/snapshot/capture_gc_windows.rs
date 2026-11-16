//! Windows garbage control for the artifacts root: the bounded, never
//! link-following removal of one staging directory and the stale-staging
//! sweep, over paths rather than directory descriptors.

use std::{ffi::OsStr, fs, io, path::Path, time::Duration};

use super::{
    SnapshotError,
    capture_dest::capture_io,
    capture_lock_windows::{RootLock, StageLock, probe_stage_lock},
};
use crate::workspace::{contain::nanos, windows_identity};

/// Per-directory enumeration and depth bounds: a staging directory holds
/// only what one capture wrote, so exceeding a bound means hostile or
/// foreign content — the removal stops and leaves the rest as garbage.
const MAX_REMOVAL_ENTRIES: usize = 4096;
const MAX_REMOVAL_DEPTH: usize = 8;
const MAX_SWEEP_ENTRIES: usize = 10_000;
const STAGING_PREFIX: &str = ".staging-";

/// Removes the staging directory named under the artifacts root: bounded
/// walk, links unlinked rather than followed, best-effort throughout.
pub(super) fn remove_tree_bounded(parent: &Path, name: &OsStr) -> io::Result<()> {
    remove_tree_with_limits(parent, name, MAX_REMOVAL_ENTRIES, MAX_REMOVAL_DEPTH)
}

/// The same bounded walk under caller-supplied limits: `max_entries` bounds
/// each directory listing and `max_depth` bounds the recursion.
pub(super) fn remove_tree_with_limits(
    parent: &Path,
    name: &OsStr,
    max_entries: usize,
    max_depth: usize,
) -> io::Result<()> {
    remove_path(&parent.join(name), max_entries, max_depth)
}

fn remove_path(path: &Path, max_entries: usize, max_depth: usize) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        // Files, links, and anything non-directory: remove by name.
        return fs::remove_file(path);
    }
    // A junction or mount point is a reparse point; remove the link itself,
    // never its target's contents.
    if windows_identity::path_is_reparse_point(path)? {
        return fs::remove_dir(path);
    }
    if max_depth == 0 {
        return Err(io::Error::other("removal depth exceeded"));
    }
    let names = list_names(path, max_entries)?;
    for name in names {
        remove_path(&path.join(name), max_entries, max_depth - 1)?;
    }
    fs::remove_dir(path)
}

fn list_names(path: &Path, max_entries: usize) -> io::Result<Vec<std::ffi::OsString>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(path)? {
        if names.len() == max_entries {
            return Err(io::Error::other("removal entry bound exceeded"));
        }
        names.push(entry?.file_name());
    }
    Ok(names)
}

/// The sweep: removes `.staging-*` siblings whose trailing nanoseconds are
/// older than `older_than` and whose lock the sweep can acquire without
/// blocking. The whole pass runs under the root-level coordination lock.
pub(super) fn sweep_stale(root: &Path, older_than: Duration) -> Result<usize, SnapshotError> {
    let _root_lock = RootLock::acquire(root)?;
    let now = nanos();
    let mut removed = 0usize;
    let entries =
        fs::read_dir(root).map_err(|error| capture_io("enumerate artifacts root", &error))?;
    for entry in entries.take(MAX_SWEEP_ENTRIES) {
        let entry = entry.map_err(|error| capture_io("enumerate artifacts root", &error))?;
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(STAGING_PREFIX) || !is_stale(&name, now, older_than)
        {
            continue;
        }
        if remove_stale_stage(root, &name) {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Removes one stale candidate only when its lock allows it. A held lock —
/// a live capture, or a lock the sweep cannot verify — is skipped and never
/// counted; a missing lock file defers to the age, which already matched.
fn remove_stale_stage(root: &Path, name: &OsStr) -> bool {
    let stage = root.join(name);
    let is_dir = fs::symlink_metadata(&stage)
        .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        .unwrap_or(false);
    if !is_dir {
        // Not a directory (link, stray) or already gone.
        return remove_tree_bounded(root, name).is_ok();
    }
    match probe_stage_lock(&stage) {
        StageLock::Held => false,
        StageLock::Free(lock) => {
            // Released before removal: Windows refuses to delete a directory
            // whose file is still locked. The root lock keeps `begin` out for
            // the whole pass, so no other capture can claim the name.
            drop(lock);
            remove_tree_bounded(root, name).is_ok()
        }
        StageLock::Absent => remove_tree_bounded(root, name).is_ok(),
    }
}

/// `.staging-<id>-<pid>-<nanos>`: the trailing nanoseconds parse from the
/// right because the id itself may contain `-` and `_`.
fn is_stale(name: &OsStr, now: u128, older_than: Duration) -> bool {
    let Some(text) = name.to_str() else {
        return false;
    };
    let Some(rest) = text.strip_prefix(STAGING_PREFIX) else {
        return false;
    };
    let Some((_, nanos_text)) = rest.rsplit_once('-') else {
        return false;
    };
    let Ok(recorded) = nanos_text.parse::<u128>() else {
        return false;
    };
    now.saturating_sub(recorded) > older_than.as_nanos()
}
