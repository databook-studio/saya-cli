//! Snapshot error vocabulary: the typed failures of selection, capture,
//! and materialization, plus the redacted io wrapper.

use std::{error::Error, fmt};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SnapshotError {
    #[error("snapshot Git metadata is malformed or unsupported")]
    GitInventory(#[from] super::super::super::git::GitInventoryError),
    #[error("the repository has no committed base for a snapshot")]
    UnbornHead,
    #[error("the repository index has unresolved conflicts")]
    ConflictedIndex,
    #[error("the snapshot contains a sensitive tracked path")]
    SensitiveTrackedPath,
    #[error("snapshot selection contains an unsafe or unsupported path")]
    UnsafePath,
    #[error("snapshot selection includes an unsupported file type")]
    UnsupportedFileType,
    #[error("snapshot policy is not bound to the selected repository")]
    PolicyRepositoryMismatch,
    #[error("snapshot selection was cancelled")]
    Cancelled,
    #[error("snapshot selection exceeded its deadline")]
    DeadlineExceeded,
    #[error("snapshot exceeds a file, count, byte, deadline, or manifest bound")]
    TooLarge,
    #[error("snapshot manifest serialization failed")]
    Serialization,
    #[error("snapshot artifact id is not a safe path component")]
    ArtifactPathInvalid,
    #[error("a snapshot with this id already exists in the artifacts root")]
    SnapshotExists,
    /// The snapshot directory was published under its id, but a durability
    /// sync failed afterwards. The id is occupied and may already look
    /// complete; it must not be silently retried or reused, and its
    /// durability is unverified.
    #[error("the snapshot was published under its id but its durability sync failed afterwards")]
    PublishedButNotDurable,
    #[error(
        "the snapshot artifacts root is not a private directory or is unsupported on this platform"
    )]
    ArtifactRootRefused,
    #[error("snapshot manifest bytes do not match their declared digest")]
    ManifestDigestMismatch,
    #[error("no published snapshot exists under this id")]
    SnapshotNotFound,
    #[error("the published snapshot manifest or completion record is malformed")]
    CapturedManifestMalformed,
    #[error("staged object bytes do not match their declared digest")]
    ObjectDigestMismatch,
    #[error("the snapshot source no longer matches the planned identity")]
    SourceIdentityChanged,
    #[error("the planned snapshot source is gone")]
    SourceDisappeared,
    #[error("a symlink appeared at the snapshot source path; capture refuses without following")]
    SourceSymlink,
    #[error("the snapshot source is not a regular file")]
    SourceNotRegularFile,
    #[error("the snapshot entry is not a working-tree source")]
    SourceNotWorkingTree,
    #[error("snapshot capture io failed: {description}")]
    CaptureIo {
        description: String,
        #[source]
        source: Option<RedactedIoError>,
    },
    #[error("the snapshot entry is not a committed Git blob source")]
    SourceNotGitBlob,
    #[error("the captured blob bytes do not match the planned declared size")]
    BlobSizeMismatch,
    #[error("the blob bytes do not match the repository's object id")]
    BlobDigestMismatch,
    #[error("the materialization target is not an empty private directory")]
    MaterializeTargetInvalid,
    #[error("materialization failed and the target could not be fully cleaned: discard the target")]
    MaterializeCleanupFailed,
}

/// A redacted wrapper around an [`std::io::Error`] for capture failures: the
/// kind and raw os code stay available, but the error's own message is never
/// rendered — a caller sink's error text may carry a path or captured bytes.
/// Debug prints only the kind and raw os code, Display is bounded and
/// path-free, and [`Error::source`] is always `None`.
pub struct RedactedIoError {
    error: std::io::Error,
}

impl RedactedIoError {
    /// Wraps `error` for redacted reporting.
    pub fn new(error: std::io::Error) -> Self {
        Self { error }
    }

    /// The wrapped error's [`std::io::ErrorKind`].
    pub fn kind(&self) -> std::io::ErrorKind {
        self.error.kind()
    }
}

impl fmt::Debug for RedactedIoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedactedIoError")
            .field("kind", &self.error.kind())
            .field("raw_os_error", &self.error.raw_os_error())
            .finish()
    }
}

impl fmt::Display for RedactedIoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = self.error.kind();
        match self.error.raw_os_error() {
            Some(code) => write!(formatter, "io error kind {kind} (os code {code})"),
            None => write!(formatter, "io error kind {kind}"),
        }
    }
}

impl Error for RedactedIoError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        // The wrapped message is caller-supplied text and is never surfaced.
        None
    }
}

impl SnapshotError {
    pub(crate) fn from_workspace_error(error: crate::HarnessError) -> Self {
        match error {
            crate::HarnessError::BoundsExceeded { .. } => Self::TooLarge,
            _ => Self::UnsafePath,
        }
    }
}
