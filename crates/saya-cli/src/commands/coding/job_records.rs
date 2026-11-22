//! A job's own records directory: the facts a reader outside a coding verb can
//! learn from it, and its one bounded removal.
//!
//! A job keeps its private state under `<state_root>/<job>/`: the request it was
//! created with, the terminal outcome block, the in-place root it wrote, its
//! events and its checkpoints. Everything here reads that directory alone — no
//! store open, no repository, no run — so a surface that is not a coding verb
//! (a session delete, say) can ask what a job is without opening one.
//!
//! The removal is this one function: a bounded walk that unlinks links instead
//! of following them, so a job's directory is removed by the owner of the layout
//! and never by a shell.

use super::{CodingError, JobOrigin, context};
use crate::state_path;
use saya_types::JobId;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// The job's request is bounded at 64 KiB where it is written
/// (`job_files::write`) and its outcome block at 256 KiB (`outcome`); a reader
/// here bounds both by the same numbers.
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_OUTCOME_BYTES: usize = 256 * 1024;

/// Per-directory entries and recursion depth the removal will walk: a job's
/// records are its own writes, so a tree past these is not one this build made
/// and is left rather than swept.
const MAX_ENTRIES: usize = 4096;
const MAX_DEPTH: usize = 12;

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

/// The facts `state_root` holds about `job`.
pub(crate) fn facts(state_root: &Path, job: &JobId) -> JobFacts {
    let records = state_root.join(job.as_str());
    let present = records.is_dir();
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
    super::root_checkout::InPlaceRootRecord::read(
        &state_root.join(job.as_str()).join("inplace-root.json"),
    )
    .is_ok()
}

/// Removes the job's whole records directory, bounded and never following a
/// link. An absent directory is already removed.
pub(crate) fn delete_records(state_root: &Path, job: &JobId) -> Result<(), CodingError> {
    if !is_one_name(job.as_str()) {
        return Err(CodingError::StateDirectory);
    }
    let path = state_root.join(job.as_str());
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CodingError::owner(error)),
        Ok(_) => remove_tree(&path, MAX_DEPTH),
    }
}

/// The state root a job's records live under: the state database's own
/// directory, beside the database the job's rows are kept in.
pub(crate) fn root() -> PathBuf {
    state_path::state_db_path()
        .parent()
        .map(|parent| parent.join("coding"))
        .unwrap_or_else(|| PathBuf::from("coding"))
}

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
