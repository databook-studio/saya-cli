//! A job's own records directory: the facts a reader outside a coding verb can
//! learn from it, and its one bounded removal.
//!
//! A job keeps its private state under `<state_root>/<job>/`: the request it was
//! created with, the terminal outcome block, the in-place root it wrote, its
//! events and its checkpoints. Everything here reads that directory alone — no
//! store open, no repository, no run — so a surface that is not a coding verb
//! (a session delete, say) can ask what a job is without opening one.
//!
//! The removal is this one function: the job's records are first renamed out of
//! the job's name to a tombstone in the job's own directory — one atomic step,
//! so a delete that stops part-way leaves a tombstone and never a half job, and
//! the next delete finishes it — and only then taken, through directory
//! descriptors on unix so a swapped link can never redirect the walk.

use super::{CodingError, JobOrigin, context};
use crate::state_path;
use saya_types::JobId;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[path = "job_records_tombstone.rs"]
mod tombstone;

/// The job's request is bounded at 64 KiB where it is written
/// (`job_files::write`) and its outcome block at 256 KiB (`outcome`); a reader
/// here bounds both by the same numbers.
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_OUTCOME_BYTES: usize = 256 * 1024;

/// Per-directory entries and recursion depth the removal will walk: a job's
/// records are its own writes, so a tree past these is not one this build made
/// and is left rather than swept.
pub(super) const MAX_ENTRIES: usize = 4096;
pub(super) const MAX_DEPTH: usize = 12;

/// The states a job never leaves: a job in one of them takes no further turn.
const TERMINAL: [&str; 3] = ["completed", "failed", "cancelled"];

/// What a job's records state about it, read without a store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct JobFacts {
    /// Where the job was opened from, as its request states it. A request that
    /// cannot be read is a `cli` job: a reader acts on what it can prove.
    pub(crate) origin: JobOrigin,
    /// Whether the job recorded a terminal state — finished, failed or
    /// cancelled. A job that never finalized (running, queued, paused,
    /// uncertain) is not terminal.
    pub(crate) terminal: bool,
    /// Whether the job's folder lease is held by a live process, this one
    /// included.
    pub(crate) lease_held: bool,
    /// Whether the job's records directory exists at all.
    pub(crate) records_present: bool,
}

/// Whether the job's records directory is present. A lookup never resolves a
/// name a deletion left: a dot-prefixed name is not a job's.
pub(crate) fn present(state_root: &Path, job: &JobId) -> bool {
    !job.as_str().starts_with('.') && state_root.join(job.as_str()).is_dir()
}

/// The facts `state_root` holds about `job`.
pub(crate) fn facts(state_root: &Path, job: &JobId) -> JobFacts {
    let records = state_root.join(job.as_str());
    let present = present(state_root, job);
    JobFacts {
        origin: origin(&records),
        terminal: present && terminal(&records),
        lease_held: present && lease_held(state_root, &records),
        records_present: present,
    }
}

/// The origin the job's own request states.
fn origin(records: &Path) -> JobOrigin {
    context::read(&records.join("request.json"), MAX_REQUEST_BYTES)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<super::CodingJobRequest>(&bytes).ok())
        .map_or(JobOrigin::Cli, |request| request.origin())
}

/// Whether the job's outcome block recorded a terminal state.
fn terminal(records: &Path) -> bool {
    context::read(&records.join("outcome.json"), MAX_OUTCOME_BYTES)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|outcome| outcome.get("state")?.as_str().map(str::to_owned))
        .is_some_and(|state| TERMINAL.contains(&state.as_str()))
}

/// Whether the folder the job's in-place root names is leased right now. A job
/// that works in a managed checkout records no in-place root and leases no
/// folder: its liveness is its own process, which a delete cannot see, so a
/// managed job is never read as held.
fn lease_held(state_root: &Path, records: &Path) -> bool {
    let Ok(record) =
        super::root_checkout::InPlaceRootRecord::read(&records.join("inplace-root.json"))
    else {
        return false;
    };
    super::in_place_lease::leased(state_root, record.root_id().as_str()).unwrap_or(true)
}

/// Whether the job works in place: it recorded the folder root it writes. A
/// job that recorded none works in a managed checkout, which runs one turn per
/// run and takes no further turn here.
pub(crate) fn works_in_place(state_root: &Path, job: &JobId) -> bool {
    present(state_root, job)
        && super::root_checkout::InPlaceRootRecord::read(
            &state_root.join(job.as_str()).join("inplace-root.json"),
        )
        .is_ok()
}

/// Removes the records of a run that ended before its first turn — the run a
/// slash command gave up, and the one a session closes while it prepared.
///
/// This is the job delete path, with the facts it reads: the job is the
/// session's own (its request states origin `session`), no live process holds
/// its folder, and the run is over — the caller proves the last one, because a
/// run that took no turn records no outcome block and so can never read as
/// `terminal` in [`JobFacts`]. Nothing is removed unless all three hold: a job
/// the CLI opened, a folder another run holds, and a name that keeps no records
/// are all left exactly as they are.
///
/// Best effort by construction — a caller giving a run up is reporting a release,
/// not running a verb — so a deletion that fails leaves the records a session's
/// own delete leaves, never a half job: the tombstone is what the walk takes.
pub(crate) fn discard_prepared(state_root: &Path, job: &JobId) -> Result<(), CodingError> {
    let facts = facts(state_root, job);
    if !facts.records_present || facts.origin.is_cli() || facts.lease_held {
        return Ok(());
    }
    delete_records(state_root, job)
}

/// Removes the job's whole records directory: renamed to a tombstone first, so a
/// stop leaves the tombstone alone, and taken afterwards through the module that
/// owns the one walk. An absent directory is already removed.
pub(crate) fn delete_records(state_root: &Path, job: &JobId) -> Result<(), CodingError> {
    if !is_one_name(job.as_str()) {
        return Err(CodingError::StateDirectory);
    }
    // A delete that stopped part-way left one tombstone and no job under any
    // name of its own; this delete finishes it before starting one of its own.
    tombstone::sweep(state_root);
    // The tombstone comes first: one rename in the job's own directory, atomic,
    // so from this moment nothing is left under the job's name whatever stops
    // next. Nothing to rename is a job already removed.
    let tombstone = tombstone::name(job.as_str());
    match fs::rename(state_root.join(job.as_str()), state_root.join(&tombstone)) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(CodingError::owner(error)),
    }
    remove_tombstone(state_root, &tombstone)
}

/// Takes the tombstoned tree on unix: through directory descriptors, so a link
/// swapped into the tree cannot redirect the walk.
#[cfg(unix)]
fn remove_tombstone(state_root: &Path, name: &str) -> Result<(), CodingError> {
    super::job_records_remove::remove(state_root, name)
}

/// Windows keeps this path walk: the platform's own answer to a reparse point is
/// `FILE_FLAG_OPEN_REPARSE_POINT` on the handles it opens, and the tombstone
/// above is what keeps a stop to one leftover entry there too.
#[cfg(not(unix))]
fn remove_tombstone(state_root: &Path, name: &str) -> Result<(), CodingError> {
    remove_tree(&state_root.join(name), MAX_DEPTH)
}

/// The state root a job's records live under: the state database's own
/// directory, beside the database the job's rows are kept in.
pub(crate) fn root() -> PathBuf {
    state_path::state_db_path()
        .parent()
        .map(|parent| parent.join("coding"))
        .unwrap_or_else(|| PathBuf::from("coding"))
}

/// The path walk a host without unix descriptors keeps: bounded, and unlinks a
/// link instead of following it.
#[cfg(not(unix))]
fn remove_tree(path: &Path, depth: usize) -> Result<(), CodingError> {
    let metadata = fs::symlink_metadata(path).map_err(CodingError::owner)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return fs::remove_file(path).map_err(CodingError::owner);
    }
    if depth == 0 {
        return Err(CodingError::StateDirectory);
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(path).map_err(CodingError::owner)? {
        if names.len() == MAX_ENTRIES {
            return Err(CodingError::StateDirectory);
        }
        names.push(entry.map_err(CodingError::owner)?.file_name());
    }
    for name in names {
        remove_tree(&path.join(name), depth - 1)?;
    }
    fs::remove_dir(path).map_err(CodingError::owner)
}

/// Whether `id` is one path name: the store's own rule for a job id, so a
/// removal can never reach a sibling.
fn is_one_name(id: &str) -> bool {
    !(id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id == "."
        || id == ".."
        || id == "inplace-leases"
        || id == "snapshots")
}

#[cfg(test)]
#[path = "job_records_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "job_records_tombstone_tests.rs"]
mod tombstone_tests;
