//! Edit-time before-images for an in-place job: the first bytes each tool
//! edit saw, kept in a bounded private store beside the job's records.
//!
//! One append-only index names the latest event per path and one object file
//! holds each recorded file's exact bytes. The store never refuses an edit for
//! a bound: past any bound it records an explicit `Unrecorded` reason and says
//! so in the tool result. An error is an I/O failure, never a policy decision.

mod index;
mod objects;
mod store;

pub use store::{BeforeImageError, BeforeImageStore};

use saya_types::Sha256Digest;
use serde::{Deserialize, Serialize};
use std::fmt;

/// The most paths one job's store will record.
pub const MAX_BEFORE_IMAGE_PATHS: usize = 4096;
/// The largest single file the store will keep.
pub const MAX_BEFORE_IMAGE_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// The most bytes all recorded files may occupy.
pub const MAX_BEFORE_IMAGE_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
/// The index's own byte bound.
pub const MAX_BEFORE_IMAGE_INDEX_BYTES: usize = 4 * 1024 * 1024;

/// What the path held when the job first touched it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
pub enum BeforeState {
    /// The path did not exist.
    Absent,
    /// The path held these bytes.
    File {
        sha256: Sha256Digest,
        size: u64,
        executable: bool,
    },
    /// The path was touched but deliberately not recorded.
    Unrecorded(UnrecordedReason),
}

/// Why a first touch was not recorded. Closed and serialized in snake case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnrecordedReason {
    /// The file was larger than [`MAX_BEFORE_IMAGE_FILE_BYTES`].
    OverFileBound,
    /// Recording the file would pass [`MAX_BEFORE_IMAGE_TOTAL_BYTES`].
    OverTotalBound,
    /// The store already held [`MAX_BEFORE_IMAGE_PATHS`] paths.
    OverPathBound,
    /// The path is credential-shaped, so no bytes are kept.
    Sensitive,
}

impl UnrecordedReason {
    /// The short, model-safe label the tool result carries.
    pub fn text(self) -> &'static str {
        match self {
            Self::OverFileBound => "file over 8 MiB",
            Self::OverTotalBound => "before-image budget exhausted",
            Self::OverPathBound => "too many recorded paths",
            Self::Sensitive => "sensitive path",
        }
    }
}

/// One path's latest before-state.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeforeImageEntry {
    pub path: String,
    pub state: BeforeState,
    pub turn: u32,
}

impl fmt::Debug for BeforeImageEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BeforeImageEntry")
            .field("state", &self.state)
            .field("turn", &self.turn)
            .finish()
    }
}

/// The outcome of recording one first touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstTouch {
    /// This call recorded the path.
    Recorded,
    /// An earlier call already recorded the path; nothing changed.
    AlreadyRecorded,
    /// The path is deliberately unrecorded, with its reason.
    Unrecorded(UnrecordedReason),
}
