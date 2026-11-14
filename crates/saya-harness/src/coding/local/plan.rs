//! The local baseline manifest: an edit-time marker with the root identity
//! and snapshot id, its validation, and the verified read-back.
//!
//! Serializing a struct (never a map) keeps the stored bytes canonical for a
//! given plan, so a verified read can require the exact canonical shape. The
//! marker holds no inventory: change detection belongs to the job's
//! before-image store (tool edits) and its Git start snapshot (shell edits).

use std::fmt;

use saya_types::{RepositoryId, Sha256Digest, SnapshotId};
use serde::{Deserialize, Serialize};

use crate::coding::snapshot::MAX_SNAPSHOT_MANIFEST_BYTES;

use super::identity::LocalRoot;
use super::plan_error::LocalBaselineError;

pub(super) const MAX_ROOT_PATH_BYTES: usize = 4096;

/// The canonical body, in fixed field order.
#[derive(Serialize, Deserialize)]
pub(super) struct LocalBaselineBody {
    pub(super) schema_version: u32,
    pub(super) mode: String,
    pub(super) snapshot_id: SnapshotId,
    pub(super) root_id: RepositoryId,
    pub(super) root_path: String,
}

pub struct LocalBaseline {
    pub(super) body: LocalBaselineBody,
    pub(super) manifest_sha256: Sha256Digest,
    pub(super) canonical_bytes: Vec<u8>,
}

impl LocalBaseline {
    /// An edit-time baseline marker: the root identity and snapshot id, with
    /// no entries. The published baseline stays so the root is still verified
    /// and the capture is still an ordinary, empty one.
    pub fn edit_time(
        root: &LocalRoot,
        snapshot_id: SnapshotId,
    ) -> Result<Self, LocalBaselineError> {
        let root_path = root
            .path()
            .to_str()
            .filter(|path| path.len() <= MAX_ROOT_PATH_BYTES)
            .ok_or(LocalBaselineError::UnsupportedRoot)?
            .to_owned();
        let body = LocalBaselineBody {
            schema_version: 1,
            mode: "in_place".to_owned(),
            snapshot_id,
            root_id: root.id().as_repository_id().clone(),
            root_path,
        };
        let canonical_bytes =
            serde_json::to_vec(&body).map_err(|_| LocalBaselineError::Serialization)?;
        if canonical_bytes.len() > MAX_SNAPSHOT_MANIFEST_BYTES {
            return Err(LocalBaselineError::ManifestBytesExceeded {
                measured: canonical_bytes.len(),
                limit: MAX_SNAPSHOT_MANIFEST_BYTES,
            });
        }
        let manifest_sha256 = crate::coding::snapshot::digest(&canonical_bytes)
            .map_err(|_| LocalBaselineError::Serialization)?;
        Ok(Self {
            body,
            manifest_sha256,
            canonical_bytes,
        })
    }

    pub fn snapshot_id(&self) -> &SnapshotId {
        &self.body.snapshot_id
    }

    pub fn repository_id(&self) -> &RepositoryId {
        &self.body.root_id
    }

    pub fn root_path(&self) -> &str {
        &self.body.root_path
    }

    pub fn manifest_sha256(&self) -> &Sha256Digest {
        &self.manifest_sha256
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    /// Reads a stored baseline back under the caller's trusted digest. The
    /// body is re-validated and re-serialized; only the canonical shape
    /// survives, so a reordered or extended manifest is refused.
    pub fn from_verified_bytes(
        bytes: &[u8],
        expected_digest: &Sha256Digest,
    ) -> Result<Self, LocalBaselineError> {
        if bytes.len() > MAX_SNAPSHOT_MANIFEST_BYTES {
            return Err(LocalBaselineError::ManifestBytesExceeded {
                measured: bytes.len(),
                limit: MAX_SNAPSHOT_MANIFEST_BYTES,
            });
        }
        if &crate::coding::snapshot::digest(bytes).map_err(|_| LocalBaselineError::Serialization)?
            != expected_digest
        {
            return Err(LocalBaselineError::DigestMismatch);
        }
        let body: LocalBaselineBody =
            serde_json::from_slice(bytes).map_err(|_| LocalBaselineError::Malformed)?;
        validate_body(&body)?;
        if serde_json::to_vec(&body).map_err(|_| LocalBaselineError::Serialization)? != bytes {
            return Err(LocalBaselineError::Malformed);
        }
        Ok(Self {
            body,
            manifest_sha256: expected_digest.clone(),
            canonical_bytes: bytes.to_vec(),
        })
    }
}

impl fmt::Debug for LocalBaseline {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalBaseline")
            .field("snapshot_id", &self.body.snapshot_id)
            .field("root_id", &self.body.root_id)
            .field("manifest_sha256", &self.manifest_sha256)
            .finish()
    }
}

fn validate_body(body: &LocalBaselineBody) -> Result<(), LocalBaselineError> {
    if body.schema_version != 1 || body.mode != "in_place" {
        return Err(LocalBaselineError::Malformed);
    }
    if body.root_path.is_empty()
        || body.root_path.len() > MAX_ROOT_PATH_BYTES
        || body.root_path.as_bytes().contains(&0)
    {
        return Err(LocalBaselineError::UnsupportedRoot);
    }
    Ok(())
}
