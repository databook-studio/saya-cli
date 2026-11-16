//! The one job baseline a native coding job carries: a Git snapshot manifest
//! or an in-place local baseline.
//!
//! Both carry the same linkage — a snapshot id, a repository/root id, a
//! canonical manifest digest and bytes — so the job and its adapters read the
//! shared accessors and only Git-only operations ask for the Git variant.
//! `from_verified_bytes` dispatches on the stored `mode` marker: a Git
//! manifest has none, and an in-place baseline states `in_place`.

use std::fmt;

use saya_types::{RepositoryId, Sha256Digest, SnapshotId};
use serde_json::Value;
use thiserror::Error;

use crate::coding::snapshot::{
    MAX_SNAPSHOT_MANIFEST_BYTES, SnapshotError, SnapshotManifest, SnapshotWarningFact,
};

use super::plan::LocalBaseline;
use super::plan_error::LocalBaselineError;

/// The stored mode marker of an in-place baseline. A Git manifest has no
/// `mode` field, so the marker also tells the two shapes apart.
const IN_PLACE_MODE: &str = "in_place";

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum JobBaselineError {
    #[error(transparent)]
    Git(#[from] SnapshotError),
    #[error(transparent)]
    InPlace(#[from] LocalBaselineError),
    #[error("the job's plan baseline is malformed")]
    Malformed,
    #[error("the job's plan baseline names an unsupported mode")]
    UnsupportedMode,
}

pub enum JobBaseline {
    /// Boxed: the snapshot manifest is several times the size of the
    /// edit-time marker beside it.
    Git(Box<SnapshotManifest>),
    InPlace(LocalBaseline),
}

impl JobBaseline {
    pub fn snapshot_id(&self) -> &SnapshotId {
        match self {
            Self::Git(manifest) => manifest.snapshot_id(),
            Self::InPlace(baseline) => baseline.snapshot_id(),
        }
    }

    pub fn repository_id(&self) -> &RepositoryId {
        match self {
            Self::Git(manifest) => manifest.repository_id(),
            Self::InPlace(baseline) => baseline.repository_id(),
        }
    }

    pub fn manifest_sha256(&self) -> &Sha256Digest {
        match self {
            Self::Git(manifest) => manifest.manifest_sha256(),
            Self::InPlace(baseline) => baseline.manifest_sha256(),
        }
    }

    /// Non-fatal facts about content the snapshot includes. An in-place
    /// baseline never has any.
    pub fn warnings(&self) -> &[SnapshotWarningFact] {
        match self {
            Self::Git(manifest) => manifest.warnings(),
            Self::InPlace(_) => &[],
        }
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        match self {
            Self::Git(manifest) => manifest.canonical_bytes(),
            Self::InPlace(baseline) => baseline.canonical_bytes(),
        }
    }

    pub fn git(&self) -> Option<&SnapshotManifest> {
        match self {
            Self::Git(manifest) => Some(manifest),
            Self::InPlace(_) => None,
        }
    }

    pub fn in_place(&self) -> Option<&LocalBaseline> {
        match self {
            Self::InPlace(baseline) => Some(baseline),
            Self::Git(_) => None,
        }
    }

    /// Reads a stored baseline back under the caller's trusted digest. The
    /// stored shape decides the variant, and the delegate re-validates and
    /// re-serializes it, so a Git manifest never becomes an in-place
    /// baseline and the reverse.
    pub fn from_verified_bytes(
        bytes: &[u8],
        expected_digest: &Sha256Digest,
    ) -> Result<Self, JobBaselineError> {
        if bytes.len() > MAX_SNAPSHOT_MANIFEST_BYTES {
            return Err(JobBaselineError::Malformed);
        }
        let value: Value =
            serde_json::from_slice(bytes).map_err(|_| JobBaselineError::Malformed)?;
        match value.get("mode").and_then(Value::as_str) {
            None => Ok(Self::Git(Box::new(SnapshotManifest::from_verified_bytes(
                bytes,
                expected_digest,
            )?))),
            Some(IN_PLACE_MODE) => Ok(Self::InPlace(LocalBaseline::from_verified_bytes(
                bytes,
                expected_digest,
            )?)),
            Some(_) => Err(JobBaselineError::UnsupportedMode),
        }
    }
}

impl fmt::Debug for JobBaseline {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (mode, entries) = match self {
            Self::Git(manifest) => ("git", manifest.entries().len()),
            Self::InPlace(_) => ("in_place", 0),
        };
        formatter
            .debug_struct("JobBaseline")
            .field("mode", &mode)
            .field("snapshot_id", self.snapshot_id())
            .field("repository_id", self.repository_id())
            .field("entry_count", &entries)
            .field("manifest_sha256", self.manifest_sha256())
            .finish()
    }
}
