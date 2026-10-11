use std::{error::Error, fmt, io::Write};

use saya_types::{GitObjectId, RepositoryId, Sha256Digest, SnapshotId};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::facts::{SnapshotChange, SnapshotStatusFact};
use super::types::*;

#[derive(Serialize)]
pub(crate) struct SnapshotManifestBody {
    pub(super) schema_version: u32,
    pub(super) snapshot_id: SnapshotId,
    pub(super) repository_id: RepositoryId,
    pub(super) common_dir_sha256: Sha256Digest,
    pub(super) repository_root: String,
    pub(super) common_dir: String,
    pub(super) base_commit: GitObjectId,
    pub(super) selection: SnapshotSelection,
    pub(super) index_sha256: Sha256Digest,
    pub(super) entries: Vec<SnapshotFileEntry>,
    pub(super) index_entries: Vec<SnapshotIndexEntry>,
    pub(super) status: Vec<SnapshotStatusFact>,
    pub(super) changes: Vec<SnapshotChange>,
    pub(super) omissions: Vec<SnapshotOmissionFact>,
}

pub struct SnapshotManifest {
    pub(crate) body: SnapshotManifestBody,
    pub(crate) manifest_sha256: Sha256Digest,
    pub(crate) canonical_bytes: Vec<u8>,
}

impl SnapshotManifest {
    pub fn snapshot_id(&self) -> &SnapshotId {
        &self.body.snapshot_id
    }

    pub fn repository_id(&self) -> &RepositoryId {
        &self.body.repository_id
    }

    pub fn common_dir_sha256(&self) -> &Sha256Digest {
        &self.body.common_dir_sha256
    }

    pub fn base_commit(&self) -> &GitObjectId {
        &self.body.base_commit
    }

    pub fn selection(&self) -> SnapshotSelection {
        self.body.selection
    }

    pub fn entries(&self) -> &[SnapshotFileEntry] {
        &self.body.entries
    }

    pub fn index_entries(&self) -> &[SnapshotIndexEntry] {
        &self.body.index_entries
    }

    pub fn status(&self) -> &[SnapshotStatusFact] {
        &self.body.status
    }

    pub fn changes(&self) -> &[SnapshotChange] {
        &self.body.changes
    }

    pub fn omissions(&self) -> &[SnapshotOmissionFact] {
        &self.body.omissions
    }

    pub fn manifest_sha256(&self) -> &Sha256Digest {
        &self.manifest_sha256
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }
}

impl fmt::Debug for SnapshotManifest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SnapshotManifest")
            .field("snapshot_id", &self.body.snapshot_id)
            .field("repository_id", &self.body.repository_id)
            .field("selection", &self.body.selection)
            .field("entry_count", &self.body.entries.len())
            .field("index_entry_count", &self.body.index_entries.len())
            .field("omission_count", &self.body.omissions.len())
            .field("manifest_sha256", &self.manifest_sha256)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SnapshotError {
    #[error("snapshot Git metadata is malformed or unsupported")]
    GitInventory(#[from] super::super::git::GitInventoryError),
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

pub(crate) fn manifest_bytes(body: &SnapshotManifestBody) -> Result<Vec<u8>, SnapshotError> {
    let mut writer = BoundedManifestWriter::new(MAX_SNAPSHOT_MANIFEST_BYTES);
    if serde_json::to_writer(&mut writer, body).is_err() {
        return Err(if writer.exceeded {
            SnapshotError::TooLarge
        } else {
            SnapshotError::Serialization
        });
    }
    Ok(writer.bytes)
}

pub(crate) fn digest(bytes: &[u8]) -> Result<Sha256Digest, SnapshotError> {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut hex, "{byte:02x}").map_err(|_| SnapshotError::Serialization)?;
    }
    Sha256Digest::parse(&hex).map_err(|_| SnapshotError::Serialization)
}

/// Lowers a streaming SHA-256 over written bytes to the typed digest, with
/// the same hex encoding as [`digest`]. Consumes the hasher, because
/// `finalize` takes it by value. Used by the object sink to verify its own
/// content identity at `finish`; the sink itself exists only on the
/// supported targets, so this helper is gated to them as well.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn digest_from_hasher(hasher: Sha256) -> Result<Sha256Digest, SnapshotError> {
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut hex, "{byte:02x}").map_err(|_| SnapshotError::Serialization)?;
    }
    Sha256Digest::parse(&hex).map_err(|_| SnapshotError::Serialization)
}

struct BoundedManifestWriter {
    bytes: Vec<u8>,
    maximum: usize,
    exceeded: bool,
}

impl BoundedManifestWriter {
    fn new(maximum: usize) -> Self {
        Self {
            bytes: Vec::new(),
            maximum,
            exceeded: false,
        }
    }
}

impl Write for BoundedManifestWriter {
    fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
        if self.bytes.len().saturating_add(input.len()) > self.maximum {
            self.exceeded = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "snapshot manifest byte bound exceeded",
            ));
        }
        self.bytes.extend_from_slice(input);
        Ok(input.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
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
