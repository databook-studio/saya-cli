//! The tombstone a job's removal leaves: the one atomic rename that moves a
//! job's records out of the job's own name before anything is taken, what such
//! a name reads as, and the sweep that finishes one a stopped delete left.
//!
//! A job is therefore either under its own name or a tombstone, never half of
//! one: a delete that stops part-way leaves one `.deleting-<id>-<nonce>` entry,
//! and the next delete takes it before starting one of its own.

use super::{MAX_ENTRIES, is_one_name, remove_tombstone};
use saya_types::JobId;
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// The prefix a deletion stamps on the name it moves a job's records to. One
/// rename inside the job's own directory is atomic, and a dot-prefixed name is
/// never a job, so a listing of jobs never shows it.
const PREFIX: &str = ".deleting-";

/// What a name directly under the job store's root is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// A job's own records directory: what a listing of jobs shows.
    Job,
    /// The tombstone a stopped delete left: what a listing never shows.
    Tombstone,
    /// Neither — the lease and snapshot directories, a name outside the job-id
    /// alphabet, or a tombstone whose own name does not state a job.
    Other,
}

/// What `name`, one entry directly under the job store's root, is.
pub(super) fn kind(name: &str) -> Kind {
    if let Some(tombstone) = name.strip_prefix(PREFIX) {
        return match tombstone.rsplit_once('-') {
            Some((id, nonce)) if is_nonce(nonce) && is_job_name(id) => Kind::Tombstone,
            _ => Kind::Other,
        };
    }
    if is_job_name(name) {
        Kind::Job
    } else {
        Kind::Other
    }
}

/// The name a deletion moves the records of `job` to. Digits only after the
/// job's own name, so [`kind`] reads the job's name back out of it.
pub(super) fn name(job: &str) -> String {
    format!("{PREFIX}{job}-{}", nonce())
}

/// Finishes what a delete that stopped part-way left: every tombstone under
/// `state_root` is taken. Best effort — one that cannot be taken now is exactly
/// the state a stop leaves, and the next delete tries it again.
pub(super) fn sweep(state_root: &Path) {
    for name in names(state_root) {
        if kind(&name) == Kind::Tombstone {
            let _ = remove_tombstone(state_root, &name);
        }
    }
}

/// The names directly under `state_root`, bounded: one sweep reads no further
/// than the walk itself may.
fn names(state_root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(state_root) else {
        return Vec::new();
    };
    let mut names = Vec::with_capacity(8);
    for entry in entries {
        if names.len() == MAX_ENTRIES {
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if let Some(name) = entry.file_name().to_str() {
            names.push(name.to_owned());
        }
    }
    names
}

/// A fresh nonce for a tombstone's name: the clock in nanoseconds and a
/// process-local count, so two deletes in one tick never share a name.
fn nonce() -> u128 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let count = u128::from(NEXT.fetch_add(1, Ordering::Relaxed));
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    nanos.saturating_mul(1_000).saturating_add(count)
}

/// Whether `name` is a job's own name: one path name inside the store's alphabet.
fn is_job_name(name: &str) -> bool {
    is_one_name(name) && JobId::parse(name).is_ok()
}

/// Whether a tombstone's trailing number is one: digits, and nothing else.
fn is_nonce(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}
