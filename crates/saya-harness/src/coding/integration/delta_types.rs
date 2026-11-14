use saya_types::Sha256Digest;
use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PathDelta {
    path: String,
    kind: DeltaKind,
    mode: crate::coding::snapshot::SnapshotFileMode,
    bytes: u64,
    content_sha256: Option<Sha256Digest>,
}

impl PathDelta {
    pub(super) fn new(
        path: String,
        kind: DeltaKind,
        mode: crate::coding::snapshot::SnapshotFileMode,
        bytes: u64,
        content_sha256: Option<Sha256Digest>,
    ) -> Self {
        Self {
            path,
            kind,
            mode,
            bytes,
            content_sha256,
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn kind(&self) -> DeltaKind {
        self.kind
    }

    /// The live file's mode; for a deletion, the baseline entry's mode.
    pub fn mode(&self) -> crate::coding::snapshot::SnapshotFileMode {
        self.mode
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn content_sha256(&self) -> Option<&Sha256Digest> {
        self.content_sha256.as_ref()
    }
}

impl fmt::Debug for PathDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PathDelta")
            .field("kind", &self.kind)
            .field("bytes", &self.bytes)
            .finish()
    }
}

/// The most not-included paths one delta names. Beyond this bound the delta
/// reports the overflow count instead, so a tree full of special files cannot
/// flood the review.
#[cfg(unix)]
pub const MAX_DELTA_NOT_INCLUDED: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotIncludedReason {
    NestedRepository,
    SpecialFile,
    UnsafePath,
    Oversize,
}

/// One path the walk could not include, and why. The path is a review fact:
/// the delta never silently drops it.
#[derive(Clone, PartialEq, Eq)]
pub struct NotIncludedPath {
    path: String,
    reason: NotIncludedReason,
}

impl NotIncludedPath {
    #[cfg(unix)]
    pub(super) fn new(path: String, reason: NotIncludedReason) -> Self {
        Self { path, reason }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn reason(&self) -> NotIncludedReason {
        self.reason
    }
}

impl fmt::Debug for NotIncludedPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NotIncludedPath")
            .field("path", &"[delta path]")
            .field("reason", &self.reason)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct CheckoutDelta {
    entries: Vec<PathDelta>,
    not_included: Vec<NotIncludedPath>,
    not_included_overflow: usize,
    changes_truncated: bool,
}

impl CheckoutDelta {
    pub(super) fn reported(
        mut entries: Vec<PathDelta>,
        not_included: Vec<NotIncludedPath>,
        not_included_overflow: usize,
        changes_truncated: bool,
    ) -> Self {
        entries.sort_unstable_by(|a, b| a.path.cmp(&b.path));
        Self {
            entries,
            not_included,
            not_included_overflow,
            changes_truncated,
        }
    }

    pub fn entries(&self) -> &[PathDelta] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn total_bytes(&self) -> u64 {
        self.entries
            .iter()
            .fold(0u64, |acc, entry| acc.saturating_add(entry.bytes))
    }

    /// Whether the delta covers every path it could represent. A truncated
    /// delta still applies its included entries; the review names the rest.
    pub fn changes_truncated(&self) -> bool {
        self.changes_truncated
    }

    pub fn not_included(&self) -> &[NotIncludedPath] {
        &self.not_included
    }

    pub fn not_included_overflow(&self) -> usize {
        self.not_included_overflow
    }
}

impl fmt::Debug for CheckoutDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckoutDelta")
            .field("count", &self.entries.len())
            .field("not_included", &self.not_included.len())
            .field("not_included_overflow", &self.not_included_overflow)
            .field("changes_truncated", &self.changes_truncated)
            .finish()
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeltaError {
    #[error("the delta computation was cancelled")]
    Cancelled,
    #[error("the delta computation exceeded its deadline")]
    DeadlineExceeded,
    #[error("the checkout holds a symlink or special file")]
    Symlink,
    #[error("the checkout holds a nested repository marker")]
    NestedRepository,
    #[error("the checkout holds an unsafe path")]
    UnsafePath,
    #[error("the checkout holds paths that differ only by case")]
    CaseFoldCollision,
    #[error("the checkout holds too many files")]
    TooManyFiles,
    #[error("a checkout file exceeds the size bound")]
    FileTooLarge,
    #[error("the checkout exceeds the total size bound")]
    TooLarge,
    #[error("the verified snapshot does not match the checkout")]
    SnapshotMismatch,
    #[error("the checkout's output root exceeds its bound")]
    OutputRootTooLarge,
    #[error("the checkout could not be read")]
    Io,
}

pub const DEFAULT_DELTA_DEADLINE: Duration = Duration::from_secs(30);

pub(super) struct LiveFile {
    pub(super) path: String,
    pub(super) mode: crate::coding::snapshot::SnapshotFileMode,
    pub(super) bytes: u64,
    pub(super) sha256: Sha256Digest,
}

/// What one live walk included and what it had to leave out. `aborted` is
/// set when a cap stopped the walk early; the included files are still a
/// truthful prefix, but deletions cannot be classified from it.
pub(super) struct WalkOutcome {
    pub(super) files: Vec<LiveFile>,
    pub(super) not_included: Vec<NotIncludedPath>,
    pub(super) not_included_overflow: usize,
    pub(super) aborted: Option<DeltaError>,
}
