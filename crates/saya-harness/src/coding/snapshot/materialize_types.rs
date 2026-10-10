//! The materialization receipt: what one successful materialization
//! reports to the binding layer. The receipt binds CONTENT digests —
//! the bytes verified while copying — and is NOT an ownership proof:
//! W05 binds `(snapshot_id, captured_manifest_sha256)` for that. The
//! plan manifest itself is not stored in the artifacts; the binding
//! layer must persist its canonical bytes (decision recorded in
//! W04M-PREFLIGHT). Paths are redacted in `Debug`.

use std::fmt;

use saya_types::{Sha256Digest, SnapshotId};

use super::types::{SnapshotFileMode, SnapshotPath};

/// One materialized file: the manifest entry's identity restated by the
/// copy that verified it. Constructed only by the materialize
/// orchestrator.
#[derive(Clone, PartialEq, Eq)]
pub struct MaterializedEntry {
    pub(crate) path: SnapshotPath,
    pub(crate) mode: SnapshotFileMode,
    pub(crate) content_sha256: Sha256Digest,
    pub(crate) bytes: u64,
}

impl MaterializedEntry {
    pub fn path(&self) -> &SnapshotPath {
        &self.path
    }

    pub fn mode(&self) -> SnapshotFileMode {
        self.mode
    }

    pub fn content_sha256(&self) -> &Sha256Digest {
        &self.content_sha256
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl fmt::Debug for MaterializedEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MaterializedEntry")
            .field("path", &"[snapshot path]")
            .field("mode", &self.mode)
            .field("content_sha256", &self.content_sha256)
            .field("bytes", &self.bytes)
            .finish()
    }
}

/// The outcome of one successful materialization: the verified captured
/// manifest's identity plus one entry per materialized file, in
/// manifest order. Constructed only by the materialize orchestrator.
#[derive(Clone, PartialEq, Eq)]
pub struct MaterializedSnapshot {
    pub(crate) snapshot_id: SnapshotId,
    pub(crate) plan_manifest_sha256: Sha256Digest,
    pub(crate) captured_manifest_sha256: Sha256Digest,
    pub(crate) entries: Vec<MaterializedEntry>,
}

impl MaterializedSnapshot {
    pub fn snapshot_id(&self) -> &SnapshotId {
        &self.snapshot_id
    }

    pub fn plan_manifest_sha256(&self) -> &Sha256Digest {
        &self.plan_manifest_sha256
    }

    pub fn captured_manifest_sha256(&self) -> &Sha256Digest {
        &self.captured_manifest_sha256
    }

    pub fn entries(&self) -> &[MaterializedEntry] {
        &self.entries
    }
}

impl fmt::Debug for MaterializedSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MaterializedSnapshot")
            .field("snapshot_id", &self.snapshot_id)
            .field("plan_manifest_sha256", &self.plan_manifest_sha256)
            .field("captured_manifest_sha256", &self.captured_manifest_sha256)
            .field("entry_count", &self.entries.len())
            .finish()
    }
}
