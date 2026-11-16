//! The integration candidate's identity and content digest.
//!
//! The candidate is a dedicated managed checkout for one job. Its identity is
//! copied from the verified baseline checkout, and its source digest is a
//! deterministic fingerprint over the sorted `(path, mode, sha256)` tree the
//! anchored walk produces.

use std::{
    fmt,
    path::Path,
    time::{Duration, Instant},
};

use saya_agent::CancellationToken;
use saya_types::{JobId, ManagedCheckoutId, Sha256Digest, SnapshotId};
use sha2::{Digest, Sha256};

use super::delta_types::{DEFAULT_DELTA_DEADLINE, DeltaError, NotIncludedReason, WalkOutcome};
use super::delta_walk::walk_checkout_with_root_git_directory;
use crate::coding::git::IgnoredPaths;
use crate::coding::worktree::VerifiedCheckout;

/// Refusals while fingerprinting the candidate tree.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CandidateError {
    #[error("the candidate tree could not be fingerprinted")]
    Walk(#[from] DeltaError),
    #[error("the candidate digest could not be encoded")]
    Digest,
}

/// The identity of one integration candidate: the job that owns it, the fresh
/// managed checkout it lives in, and the baseline (snapshot and plan digests)
/// it was created from.
///
/// The [`fmt::Debug`] output carries the checkout id only — never a digest
/// and never a filesystem path.
#[derive(Clone, PartialEq, Eq)]
pub struct CandidateIdentity {
    job_id: JobId,
    checkout_id: ManagedCheckoutId,
    snapshot_id: SnapshotId,
    snapshot_sha256: Sha256Digest,
    plan_sha256: Sha256Digest,
}

impl CandidateIdentity {
    /// Copies the baseline identity out of an already-verified checkout. The
    /// candidate checkout id is whatever the verified record carries: the
    /// orchestrator verifies the fresh candidate id at use time and passes
    /// that verification in.
    pub fn from_verified(job_id: &JobId, verified: &VerifiedCheckout) -> Self {
        Self {
            job_id: job_id.clone(),
            checkout_id: verified.checkout_id().clone(),
            snapshot_id: verified.snapshot_id().clone(),
            snapshot_sha256: verified.snapshot_sha256().clone(),
            plan_sha256: verified.plan_sha256().clone(),
        }
    }

    pub fn job_id(&self) -> &JobId {
        &self.job_id
    }

    pub fn checkout_id(&self) -> &ManagedCheckoutId {
        &self.checkout_id
    }

    pub fn snapshot_id(&self) -> &SnapshotId {
        &self.snapshot_id
    }

    pub fn snapshot_sha256(&self) -> &Sha256Digest {
        &self.snapshot_sha256
    }

    pub fn plan_sha256(&self) -> &Sha256Digest {
        &self.plan_sha256
    }
}

impl fmt::Debug for CandidateIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CandidateIdentity")
            .field("checkout_id", &self.checkout_id.as_str())
            .finish()
    }
}

/// Digests the candidate checkout's current tree.
///
/// The digest is content-only and deterministic: independent of write order,
/// timestamps, the `.git` pointer file, and paths the checkout's Git ignore
/// rules exclude. It is the candidate SOURCE digest that later packets bind
/// evidence to. A bound trip, cancellation, or deadline is an error — never a
/// partial digest; a path the walk reports as not included is outside the
/// delta's included set and is outside the digest too.
pub fn candidate_source_digest(
    root: &Path,
    ignored: &IgnoredPaths,
    cancellation: &CancellationToken,
    deadline: Duration,
) -> Result<Sha256Digest, CandidateError> {
    source_digest(root, ignored, cancellation, deadline, false, false)
}

/// Digests a managed candidate strictly: the same tree the managed review
/// includes, but any path the walk cannot represent refuses the digest. The
/// Node runtime's managed source copy keeps this stricter identity.
pub fn managed_candidate_source_digest(
    root: &Path,
    ignored: &IgnoredPaths,
    cancellation: &CancellationToken,
    deadline: Duration,
) -> Result<Sha256Digest, CandidateError> {
    source_digest(root, ignored, cancellation, deadline, false, true)
}

/// Digests an ordinary operator project while retaining the same anchored
/// walk, canonical path/mode/content representation, and safety bounds. Only
/// the exact root `.git` directory or pointer is excluded; nested repository
/// markers, special files, and case-folded paths still refuse the walk.
pub fn operator_project_source_digest(
    root: &Path,
    ignored: &IgnoredPaths,
    cancellation: &CancellationToken,
    deadline: Duration,
) -> Result<Sha256Digest, CandidateError> {
    source_digest(root, ignored, cancellation, deadline, true, true)
}

fn source_digest(
    root: &Path,
    ignored: &IgnoredPaths,
    cancellation: &CancellationToken,
    deadline: Duration,
    allow_root_git_directory: bool,
    strict: bool,
) -> Result<Sha256Digest, CandidateError> {
    let limit = Instant::now()
        .checked_add(deadline)
        .unwrap_or_else(|| Instant::now() + DEFAULT_DELTA_DEADLINE);
    let WalkOutcome {
        files,
        not_included,
        not_included_overflow,
        aborted,
    } = walk_checkout_with_root_git_directory(
        root,
        cancellation,
        limit,
        allow_root_git_directory,
        &std::collections::HashSet::new(),
        ignored,
    )?;
    if let Some(error) = aborted {
        return Err(CandidateError::Walk(error));
    }
    if strict && (!not_included.is_empty() || not_included_overflow > 0) {
        let error = not_included
            .first()
            .map(|item| match item.reason() {
                NotIncludedReason::NestedRepository => DeltaError::NestedRepository,
                NotIncludedReason::SpecialFile => DeltaError::Symlink,
                NotIncludedReason::UnsafePath => DeltaError::UnsafePath,
                NotIncludedReason::Oversize => DeltaError::FileTooLarge,
            })
            .unwrap_or(DeltaError::Symlink);
        return Err(CandidateError::Walk(error));
    }
    digest_files(files)
}

fn digest_files(files: Vec<super::delta_types::LiveFile>) -> Result<Sha256Digest, CandidateError> {
    let mut hasher = Sha256::new();
    for file in &files {
        hasher.update(file.path.as_bytes());
        hasher.update([0u8]);
        let mode_tag: &[u8] = match file.mode {
            crate::coding::snapshot::SnapshotFileMode::Executable => MODE_EXECUTABLE,
            crate::coding::snapshot::SnapshotFileMode::Symlink => MODE_SYMLINK,
            crate::coding::snapshot::SnapshotFileMode::Regular => MODE_REGULAR,
        };
        hasher.update(mode_tag);
        hasher.update([0u8]);
        hasher.update(file.sha256.as_str().as_bytes());
        hasher.update(b"\n");
    }
    let hex = hex_lower(&hasher.finalize());
    Sha256Digest::parse(&hex).map_err(|_| CandidateError::Digest)
}

const MODE_EXECUTABLE: &[u8; 4] = b"0755";
const MODE_REGULAR: &[u8; 4] = b"0644";
const MODE_SYMLINK: &[u8; 6] = b"120000";

fn hex_lower(bytes: &[u8]) -> String {
    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        hex.push(HEX_DIGITS[(byte & 0x0f) as usize] as char);
    }
    hex
}
