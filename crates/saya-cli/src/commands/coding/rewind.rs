//! Checkpoints of one coding job's working checkout, and the operator rewind
//! over them.
//!
//! A checkpoint is the working checkout's own changed-file state at one turn
//! boundary: the paths the job's tools changed against its baseline, and the
//! bytes each of those paths held then. The job's root loop takes one at every
//! turn boundary where that state moved since the last checkpoint — never for a
//! turn that changed nothing — and the operator can put the checkout back to any
//! of them. It is a recovery convenience, never authority: nothing here grants a
//! job anything it did not already have, every restored path is one the job's
//! own write scope already admits, and a path outside that scope is never
//! touched.
//!
//! What a checkpoint covers is exactly what the job's file tools wrote in its
//! own checkout. It never represents a command, a download, a database effect,
//! or anything outside the checkout, so a rewind undoes file content and
//! nothing else.
//!
//! Every bound is named and enforced here: [`MAX_REWIND_CHECKPOINTS`] per job
//! (the oldest dropped first), [`MAX_REWIND_CHECKPOINT_BYTES`] and
//! [`MAX_REWIND_FILES_PER_CHECKPOINT`] per checkpoint, [`MAX_REWIND_FILE_BYTES`]
//! per file, and [`MAX_REWIND_RECORD_BYTES`] for the record itself. A capture
//! that cannot fit records nothing at all rather than a partial checkpoint, and
//! a rewind to a turn with no checkpoint is refused by name.

mod apply;
mod apply_baseline;
mod capture;
mod model;
mod operation;
mod sink;
mod store;

#[cfg(all(test, unix))]
#[path = "rewind/tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "rewind/in_place_fixture.rs"]
mod in_place_fixture;

#[cfg(all(test, unix))]
#[path = "rewind/in_place_tests.rs"]
mod in_place_tests;

#[cfg(all(test, unix))]
#[path = "rewind/baseline_tests.rs"]
mod baseline_tests;

#[cfg(all(test, unix))]
#[path = "rewind/job_tests.rs"]
mod job_tests;

#[cfg(all(test, unix))]
#[path = "rewind/model_tests.rs"]
mod model_tests;

use std::path::Path;

pub(crate) use operation::target;
pub(crate) use sink::RewindSink;
pub(crate) use store::RewindStore;

/// How many checkpoints one job keeps. A capture past this drops the oldest
/// checkpoint first, so the newest state is always rewound to.
pub(crate) const MAX_REWIND_CHECKPOINTS: usize = 32;

/// The most image bytes one checkpoint may hold. A turn whose changed files
/// exceed it is not checkpointed at all.
pub(crate) const MAX_REWIND_CHECKPOINT_BYTES: u64 = 16 * 1024 * 1024;

/// The most paths one checkpoint may hold.
pub(crate) const MAX_REWIND_FILES_PER_CHECKPOINT: usize = 512;

/// The most bytes one checkpointed file may hold — the whole-file write bound
/// the workspace tools already carry.
pub(crate) const MAX_REWIND_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// The serialized bound of one checkpoint record.
pub(crate) const MAX_REWIND_RECORD_BYTES: usize = 64 * 1024;

/// One checkpointed path as the listing reports it: the path, and the version
/// it held at that checkpoint — `None` when the path was absent then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RewindEntry {
    pub(crate) path: String,
    pub(crate) sha256: Option<String>,
    pub(crate) size: u64,
}

/// One checkpoint: the turn it was taken at and the state it recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RewindCheckpoint {
    /// The checkpoint's number, 1-based and ascending in the order the job took
    /// them. This is what `--to` names.
    pub(crate) turn: u32,
    pub(crate) created_unix_ms: i64,
    pub(crate) entries: Vec<RewindEntry>,
}

/// One path and the bytes it held at a checkpoint, as the restore consumes it.
/// `None` records an absent path — restoring it means removing the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapturedPath {
    pub(crate) path: String,
    pub(crate) bytes: Option<Vec<u8>>,
}

/// One path a rewind changed, and the version it holds: `None` for a path
/// the rewind removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RestoredPath {
    pub(crate) path: String,
    pub(crate) sha256: Option<String>,
    pub(crate) size: u64,
}

/// What one rewind changed in the working checkout.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct RestoreReport {
    /// Paths whose content was written back to the checkpointed version.
    pub(crate) restored: usize,
    /// Paths created after the checkpoint, removed to match it.
    pub(crate) removed: usize,
    /// Paths the checkpoint holds that the job's write scope does not admit.
    /// Never touched, and reported rather than silently skipped.
    pub(crate) out_of_scope: Vec<String>,
    /// Paths whose current version the job's own journal does not account for.
    /// Never touched unless the operator forces the rewind, and reported so
    /// the concurrent edit is not silently lost.
    pub(crate) foreign: Vec<String>,
    /// Paths whose before-state was deliberately not recorded, so no restore
    /// is possible. Never touched, and reported rather than guessed at.
    pub(crate) not_restorable: Vec<String>,
    /// Every path this rewind changed, in the order it was handled, with the
    /// version it holds.
    pub(crate) changes: Vec<RestoredPath>,
}

/// The store root for one job: a private `rewind` directory beside the job's
/// own records. Both adapters and the job's own capture resolve the same path
/// from the same job id.
pub(crate) fn store_root(records: &Path) -> std::path::PathBuf {
    records.join("rewind")
}
