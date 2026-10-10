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

use super::delta_types::{DEFAULT_DELTA_DEADLINE, DeltaError};
use super::delta_walk::walk_checkout;
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
/// timestamps, and the `.git` pointer file. It is the candidate SOURCE digest
/// that later packets bind evidence to. Any symlink, nested repository, bound
/// trip, cancellation, or deadline is an error — never a partial digest.
pub fn candidate_source_digest(
    root: &Path,
    cancellation: &CancellationToken,
    deadline: Duration,
) -> Result<Sha256Digest, CandidateError> {
    let limit = Instant::now()
        .checked_add(deadline)
        .unwrap_or_else(|| Instant::now() + DEFAULT_DELTA_DEADLINE);
    let files = walk_checkout(root, cancellation, limit)?;
    let mut hasher = Sha256::new();
    for file in &files {
        hasher.update(file.path.as_bytes());
        hasher.update([0u8]);
        hasher.update(if file.executable {
            MODE_EXECUTABLE
        } else {
            MODE_REGULAR
        });
        hasher.update([0u8]);
        hasher.update(file.sha256.as_str().as_bytes());
        hasher.update(b"\n");
    }
    let hex = hex_lower(&hasher.finalize());
    Sha256Digest::parse(&hex).map_err(|_| CandidateError::Digest)
}

const MODE_EXECUTABLE: &[u8; 4] = b"0755";
const MODE_REGULAR: &[u8; 4] = b"0644";

fn hex_lower(bytes: &[u8]) -> String {
    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        hex.push(HEX_DIGITS[(byte & 0x0f) as usize] as char);
    }
    hex
}
