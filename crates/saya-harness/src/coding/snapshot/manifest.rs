use std::{fmt, io::Write};

use saya_types::{GitObjectId, RepositoryId, Sha256Digest, SnapshotId};
use serde::Serialize;
use sha2::{Digest, Sha256};

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
