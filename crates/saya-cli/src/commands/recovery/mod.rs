//! Surface-neutral recovery operations over the session's recorded edit
//! checkpoints: listing every recorded edit with its current restore
//! status, previewing a restore as a bounded in-house diff, and restoring
//! through the guarded workspace operations. Typed outcomes and plain
//! text only — no slash command, no UI. E08 wires this into the `/recovery`
//! surface and removes this module's test-only gate.

mod diff;
mod hunks;
mod line_diff;
mod ops;
mod render;
mod render_outcome;
mod restore;

pub(crate) use diff::{diff_change, render_diff};
pub(crate) use ops::{list_changes, sha256_hex};
pub(crate) use render::render_changes;
pub(crate) use render_outcome::{render_diff_outcome, render_restore};
pub(crate) use restore::restore_change;

use saya_types::{CheckpointState, CheckpointUsage, EditSource, FileVersion, RestoreEligibility};

/// The restore status of one recorded edit as of now. An unreadable file
/// is unknown — never guessed as changed or missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StatusNow {
    Known(RestoreEligibility),
    /// The file could not be read right now.
    Unreadable,
    /// Beyond the per-call refresh bound: the file was not read.
    NotChecked,
}

/// The recorded before-side of an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BeforeSummary {
    /// The edit created the file; restoring it means removal.
    Absent,
    Present {
        size: u64,
    },
}

impl BeforeSummary {
    pub(crate) fn of(before: &saya_types::BeforeImage) -> Self {
        match before {
            saya_types::BeforeImage::Absent => Self::Absent,
            saya_types::BeforeImage::Present(version) => Self::Present { size: version.size },
            _ => Self::Absent,
        }
    }
}

/// One recorded edit of the session as the listing shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChangeItem {
    pub(crate) id: String,
    pub(crate) path: String,
    pub(crate) source: EditSource,
    pub(crate) state: CheckpointState,
    pub(crate) before: BeforeSummary,
    pub(crate) after: Option<FileVersion>,
    pub(crate) created_unix_ms: i64,
    pub(crate) restore: StatusNow,
}

/// The listing of one session's recorded edits against its quotas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChangesView {
    pub(crate) items: Vec<ChangeItem>,
    pub(crate) usage: CheckpointUsage,
    pub(crate) max_edits: u32,
    pub(crate) max_bytes: u64,
}

/// Why a restore was refused; every variant renders as a pinned sentence
/// and none of them changed the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RestoreRefusal {
    NotFound,
    NoWorkspace,
    NotEligible { status: StatusNow, path: String },
    BeforeImageUnavailable,
    BeforeImageCorrupt,
    ChangedWhileRestoring,
    Failed(String),
}

/// The typed outcome of one restore attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RestoreOutcome {
    /// The file was restored; `removed_checkpoint` is false when the
    /// checkpoint could not be consumed after the restore landed.
    Restored {
        id: String,
        path: String,
        removed_checkpoint: bool,
    },
    Refused(RestoreRefusal),
}

/// The typed outcome of one restore preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiffOutcome {
    /// A bounded unified-style preview; `truncated` says the output cap
    /// cut it short.
    Diff {
        text: String,
        truncated: bool,
    },
    BinaryOrNonText {
        before_bytes: usize,
        current_bytes: usize,
    },
    TooLargeToDiff {
        before_bytes: usize,
        current_bytes: usize,
    },
    /// The file already matches the recorded before state, line for line.
    NothingToDiff,
    Refused(RestoreRefusal),
}

#[cfg(test)]
#[path = "diff_tests.rs"]
mod diff_tests;
#[cfg(test)]
#[path = "ops_tests.rs"]
mod ops_tests;
#[cfg(test)]
#[path = "render_tests.rs"]
mod render_tests;
#[cfg(test)]
#[path = "restore_tests.rs"]
mod restore_tests;
#[cfg(test)]
mod testing;
