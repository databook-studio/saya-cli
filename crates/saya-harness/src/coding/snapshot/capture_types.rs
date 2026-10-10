//! The capture result types: what one completed snapshot capture reports to
//! the binding layer. Paths are redacted in `Debug` — only counts, modes,
//! sources, and digests ever print, matching the plan types' redaction.

use std::fmt;

use saya_types::{RepositoryId, Sha256Digest, SnapshotId};
use serde::Serialize;

use super::types::{SnapshotContentSource, SnapshotFileMode, SnapshotPath};

/// One selected content file captured into the artifacts root.
///
/// `content_sha256` is the digest of the bytes that were actually written:
/// for a working-tree source it is the same-open streaming digest, for a
/// Git blob it is the recomputed object digest. It identifies content, not
/// path or authorship.
/// Constructed only by the capture orchestrator (W04d).
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct CapturedEntry {
    pub(crate) path: SnapshotPath,
    pub(crate) mode: SnapshotFileMode,
    pub(crate) content_sha256: Sha256Digest,
    pub(crate) captured_bytes: u64,
    pub(crate) source: SnapshotContentSource,
}

impl CapturedEntry {
    pub fn path(&self) -> &SnapshotPath {
        &self.path
    }

    pub fn mode(&self) -> SnapshotFileMode {
        self.mode
    }

    pub fn content_sha256(&self) -> &Sha256Digest {
        &self.content_sha256
    }

    pub fn captured_bytes(&self) -> u64 {
        self.captured_bytes
    }

    pub fn source(&self) -> &SnapshotContentSource {
        &self.source
    }
}

impl fmt::Debug for CapturedEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapturedEntry")
            .field("path", &"[snapshot path]")
            .field("mode", &self.mode)
            .field("content_sha256", &self.content_sha256)
            .field("captured_bytes", &self.captured_bytes)
            .field("source", &self.source)
            .finish()
    }
}

/// The outcome of one completed capture: the plan linkage W04 carried
/// verbatim plus the captured-manifest identity the binding layer (W05)
/// must bind. `captured_manifest_sha256` is the digest of the serialized
/// captured manifest stored in the artifacts root — the plan digest is a
/// different field and never substitutes for it.
/// Constructed only by the capture orchestrator (W04d).
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct CapturedSnapshot {
    pub(crate) snapshot_id: SnapshotId,
    pub(crate) repository_id: RepositoryId,
    pub(crate) plan_manifest_sha256: Sha256Digest,
    pub(crate) captured_manifest_sha256: Sha256Digest,
    pub(crate) entries: Vec<CapturedEntry>,
}

impl CapturedSnapshot {
    pub fn snapshot_id(&self) -> &SnapshotId {
        &self.snapshot_id
    }

    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    pub fn plan_manifest_sha256(&self) -> &Sha256Digest {
        &self.plan_manifest_sha256
    }

    pub fn captured_manifest_sha256(&self) -> &Sha256Digest {
        &self.captured_manifest_sha256
    }

    pub fn entries(&self) -> &[CapturedEntry] {
        &self.entries
    }
}

impl fmt::Debug for CapturedSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CapturedSnapshot")
            .field("snapshot_id", &self.snapshot_id)
            .field("repository_id", &self.repository_id)
            .field("plan_manifest_sha256", &self.plan_manifest_sha256)
            .field("captured_manifest_sha256", &self.captured_manifest_sha256)
            .field("entry_count", &self.entries.len())
            .finish()
    }
}
