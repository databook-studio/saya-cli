//! Evidence binding: verify-at-use, fail closed.
//!
//! A binding pins either a managed candidate's whole-tree content digest or an
//! edit-time job's exact changed-path list at the moment evidence is produced.
//! Re-deriving the digest at use time and comparing is what makes ANY edit
//! invalidate the evidence: one flipped byte changes the digest.
//!
//! The binding is verify-at-use and fails closed: only an exact digest match
//! passes. Any anomaly — symlink, nested repository, bound trip, cancellation,
//! deadline, missing tree — is [`EvidenceError::Unverifiable`], never "assume
//! valid". Bindings are not persisted: a restart forgets them, and nothing
//! remembered means nothing trusted.

use std::{collections::BTreeMap, fmt, path::Path, time::Duration};

use saya_agent::CancellationToken;
use saya_types::{ManagedCheckoutId, Sha256Digest};
use sha2::{Digest, Sha256};

use crate::coding::git::IgnoredPaths;
use crate::coding::worktree::VerifiedCheckout;

use super::{CandidateError, candidate_source_digest};

/// Binds evidence (a check receipt, a review, or an accepted
/// run) to the exact candidate source digest at bind time.
///
/// The [`fmt::Debug`] output carries the checkout id only — never a digest and
/// never a filesystem path. Callers that need the digest call
/// [`EvidenceBinding::source_sha256`].
#[derive(Clone, PartialEq, Eq)]
pub struct EvidenceBinding {
    candidate_checkout_id: ManagedCheckoutId,
    source_sha256: Sha256Digest,
}

impl EvidenceBinding {
    /// Digests a managed candidate's whole tree at bind time via
    /// [`candidate_source_digest`].
    pub fn bind(
        candidate: &VerifiedCheckout,
        ignored: &IgnoredPaths,
        cancellation: &CancellationToken,
        deadline: Duration,
    ) -> Result<Self, EvidenceError> {
        let root = Path::new(candidate.path().as_str());
        let source_sha256 = candidate_source_digest(root, ignored, cancellation, deadline)?;
        Ok(Self {
            candidate_checkout_id: candidate.checkout_id().clone(),
            source_sha256,
        })
    }

    /// Binds an edit-time job to its exact changed-path list: each path with
    /// the digest it holds now, or `None` when it is absent. The digest is
    /// over the path-sorted `(path, sha256 | "absent")` list, so any edit, any
    /// creation and any removal changes it.
    pub fn bind_paths(
        candidate: &VerifiedCheckout,
        files: &[(String, Option<Sha256Digest>)],
    ) -> Result<Self, EvidenceError> {
        Ok(Self {
            candidate_checkout_id: candidate.checkout_id().clone(),
            source_sha256: paths_digest(files)?,
        })
    }

    pub fn candidate_checkout_id(&self) -> &ManagedCheckoutId {
        &self.candidate_checkout_id
    }

    pub fn source_sha256(&self) -> &Sha256Digest {
        &self.source_sha256
    }

    /// Re-derives the whole-tree digest of the candidate at `candidate_root`
    /// and compares. Only exact equality passes.
    pub fn verify_at_use(
        &self,
        candidate_root: &Path,
        ignored: &IgnoredPaths,
        cancellation: &CancellationToken,
        deadline: Duration,
    ) -> Result<(), EvidenceError> {
        let current = candidate_source_digest(candidate_root, ignored, cancellation, deadline)?;
        if current != self.source_sha256 {
            return Err(EvidenceError::Stale);
        }
        Ok(())
    }

    /// Re-derives the changed-path digest and compares. Mismatch is
    /// [`EvidenceError::Stale`]; only exact equality passes.
    pub fn verify_paths(
        &self,
        files: &[(String, Option<Sha256Digest>)],
    ) -> Result<(), EvidenceError> {
        if paths_digest(files)? != self.source_sha256 {
            return Err(EvidenceError::Stale);
        }
        Ok(())
    }
}

impl fmt::Debug for EvidenceBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EvidenceBinding")
            .field("checkout_id", &self.candidate_checkout_id.as_str())
            .finish()
    }
}

/// The digest over the path-sorted `(path, sha256 | "absent")` list. A path
/// named twice is hashed once, with the last digest the caller stated.
fn paths_digest(files: &[(String, Option<Sha256Digest>)]) -> Result<Sha256Digest, EvidenceError> {
    let mut sorted: BTreeMap<&str, Option<&Sha256Digest>> = BTreeMap::new();
    for (path, digest) in files {
        sorted.insert(path.as_str(), digest.as_ref());
    }
    let mut hasher = Sha256::new();
    for (path, digest) in sorted {
        hasher.update(path.as_bytes());
        hasher.update([0u8]);
        hasher.update(digest.map_or("absent", Sha256Digest::as_str).as_bytes());
        hasher.update(b"\n");
    }
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Sha256Digest::parse(&hex).map_err(|_| EvidenceError::Digest)
}

/// Why a binding could not be honored at use time.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum EvidenceError {
    #[error("the bound candidate tree changed after the evidence was produced")]
    Stale,
    #[error("the bound candidate tree could not be fingerprinted")]
    Unverifiable(#[from] CandidateError),
    #[error("the bound path list could not be encoded")]
    Digest,
}
