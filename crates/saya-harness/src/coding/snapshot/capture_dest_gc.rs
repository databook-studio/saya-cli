//! Garbage control for the artifacts root: the bounded, never
//! link-following removal of one staging directory (the cancel and error
//! path) and the stale-staging sweep (recovery from a killed capture).
//! Whatever this module cannot remove stays as inert `.staging-*` garbage
//! that never blocks a new capture and never presents as complete.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::{
    ffi::{OsStr, OsString},
    fs, io,
    os::{
        fd::{AsRawFd, RawFd},
        unix::ffi::OsStrExt,
    },
    time::Duration,
};

use super::{
    SnapshotError,
    capture_dest::capture_io,
    capture_dest_private::c_name,
    capture_lock::{StageLock, probe_stage_lock},
};
use crate::workspace::{
    contain::nanos,
    fd::{DirStream, open_dir_at, stat_at, unlink_at},
};

/// Per-directory enumeration and depth bounds: a staging directory holds
/// only what one capture wrote, so exceeding a bound means hostile or
/// foreign content — the removal stops and leaves the rest as garbage.
const MAX_REMOVAL_ENTRIES: usize = 4096;
const MAX_REMOVAL_DEPTH: usize = 8;
const MAX_SWEEP_ENTRIES: usize = 10_000;
const STAGING_PREFIX: &str = ".staging-";

/// Removes the staging directory named under the artifacts root: bounded
/// walk, symlinks unlinked rather than followed, best-effort throughout.
pub(super) fn remove_tree_bounded(parent: &fs::File, name: &OsStr) -> io::Result<()> {
    remove_tree_with_limits(parent, name, MAX_REMOVAL_ENTRIES, MAX_REMOVAL_DEPTH)
}

/// The same bounded walk under caller-supplied limits: `max_entries`
/// bounds each directory listing and `max_depth` bounds the recursion, so
/// a reset caller can cover a snapshot-sized tree without relaxing the
/// staging bounds above.
pub(super) fn remove_tree_with_limits(
    parent: &fs::File,
    name: &OsStr,
    max_entries: usize,
    max_depth: usize,
) -> io::Result<()> {
    let stat = stat_at(parent.as_raw_fd(), name)?;
    if stat.is_symlink() || !stat.is_dir() {
        // A link or stray named like staging goes by name, never opened.
        return unlink_at(parent.as_raw_fd(), name);
    }
    let dir = fs::File::from(open_dir_at(parent.as_raw_fd(), name)?);
    remove_contents(&dir, max_entries, max_depth)?;
    remove_dir_at(parent.as_raw_fd(), name)
}

/// One directory level, in three phases. Phase 1 lists the directory
/// completely before anything is removed: removing entries while reading
/// the same readdir stream is unspecified by POSIX and can skip entries,
/// which would make the final removal of the directory itself fail.
/// Phase 2 removes every collected entry. Phase 3 re-lists the directory
/// once and requires it to be empty, so the caller's removal of the
/// directory itself never races with leftovers.
fn remove_contents(dir: &fs::File, max_entries: usize, depth: usize) -> io::Result<()> {
    if depth == 0 {
        return Ok(());
    }
    let entries = list_entries(dir, max_entries)?;
    for (name, is_dir) in entries {
        if !is_dir {
            // Files, links, and anything non-directory: unlink by name.
            let _ = unlink_at(dir.as_raw_fd(), &name);
            continue;
        }
        let child = fs::File::from(open_dir_at(dir.as_raw_fd(), &name)?);
        remove_contents(&child, max_entries, depth - 1)?;
        remove_dir_at(dir.as_raw_fd(), &name)?;
    }
    require_empty(dir)
}

/// Phase 1: the complete listing, bounded. Exceeding `max_entries`
/// without reaching the end of the stream is the same ENOTEMPTY error the
/// old over-bound removal produced when the caller's unlinkat then failed.
fn list_entries(dir: &fs::File, max_entries: usize) -> io::Result<Vec<(OsString, bool)>> {
    let mut stream = DirStream::open_at(dir.as_raw_fd(), OsStr::new("."))?;
    let mut entries = Vec::new();
    while let Some((name, kind)) = stream.next()? {
        if entries.len() == max_entries {
            return Err(not_empty_error());
        }
        let is_dir = match kind {
            libc::DT_DIR => true,
            // Links and strays go by name; an unreadable entry stays.
            libc::DT_UNKNOWN => match stat_at(dir.as_raw_fd(), &name) {
                Ok(stat) => stat.is_dir(),
                Err(_) => false,
            },
            _ => false,
        };
        entries.push((name, is_dir));
    }
    Ok(entries)
}

/// Phase 3: one fresh listing; anything left means the removal did not
/// complete and the directory stays as garbage, exactly as the caller's
/// failing unlinkat would have left it.
fn require_empty(dir: &fs::File) -> io::Result<()> {
    let mut stream = DirStream::open_at(dir.as_raw_fd(), OsStr::new("."))?;
    if stream.next()?.is_some() {
        return Err(not_empty_error());
    }
    Ok(())
}

/// The raw OS error a directory removal reports when it is not empty —
/// kind `DirectoryNotEmpty` — the file's existing error convention.
fn not_empty_error() -> io::Error {
    io::Error::from_raw_os_error(libc::ENOTEMPTY)
}

/// The sweep: removes `.staging-*` siblings whose trailing nanoseconds are
/// older than `older_than` and whose advisory lock the sweep can acquire
/// without blocking. The whole pass runs under the root-level coordination
/// lock, so no `begin` — here or in another process — can be between its
/// staging `mkdirat` and its stage lock acquisition while names are being
/// scanned and removed. The id and pid segments are not interpreted — only
/// the age and the lock decide, so a malformed name simply stays as
/// garbage.
pub(super) fn sweep_stale(
    root_dir: &fs::File,
    older_than: Duration,
) -> Result<usize, SnapshotError> {
    // Held for the whole scan-and-remove pass; dropped after it.
    let _root_lock = super::capture_lock::RootLock::acquire(root_dir)?;
    let mut stream = DirStream::open_at(root_dir.as_raw_fd(), OsStr::new("."))
        .map_err(|error| capture_io("enumerate artifacts root", &error))?;
    let now = nanos();
    let mut removed = 0usize;
    for _ in 0..MAX_SWEEP_ENTRIES {
        let Some((name, _)) = stream
            .next()
            .map_err(|error| capture_io("enumerate artifacts root", &error))?
        else {
            break;
        };
        if !name.as_bytes().starts_with(STAGING_PREFIX.as_bytes())
            || !is_stale(&name, now, older_than)
        {
            continue;
        }
        if remove_stale_stage(root_dir, &name) {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Removes one stale candidate only when its lock allows it. A held lock —
/// a live capture, or a lock the sweep cannot verify — is skipped and never
/// counted; a lock the sweep acquires stays held until the candidate is
/// gone; a missing lock file defers to the age, which already matched. With
/// the root-level lock serializing creation against sweeping, a stage
/// without a lock file can only be a crashed leftover, so the age alone
/// deciding is exactly right.
fn remove_stale_stage(root_dir: &fs::File, name: &OsStr) -> bool {
    let stage = match open_dir_at(root_dir.as_raw_fd(), name) {
        Ok(dir) => fs::File::from(dir),
        // Not a directory (link, stray) or already gone: the tree remover
        // unlinks the former by name and the latter needs nothing.
        Err(_) => return remove_tree_bounded(root_dir, name).is_ok(),
    };
    match probe_stage_lock(&stage) {
        StageLock::Held => false,
        StageLock::Free(lock) => {
            let removed = remove_tree_bounded(root_dir, name).is_ok();
            drop(lock);
            removed
        }
        StageLock::Absent => remove_tree_bounded(root_dir, name).is_ok(),
    }
}

/// `.staging-<id>-<pid>-<nanos>`: the trailing nanoseconds parse from the
/// right because the id itself may contain `-` and `_`. The age is
/// `now.saturating_sub(recorded)`, so it is deterministic for a given
/// `now` and `older_than`: a value in the future — clock stepped backward,
/// or skew between the writer's clock and the sweeper's — reads as age
/// zero and is never stale, and a clock step forward ages every recorded
/// value at once.
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

/// Removes a directory entry by name once its contents are gone.
fn remove_dir_at(dir_fd: RawFd, name: &OsStr) -> io::Result<()> {
    let c = c_name(name)?;
    let rc = unsafe { libc::unlinkat(dir_fd, c.as_ptr(), libc::AT_REMOVEDIR) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
