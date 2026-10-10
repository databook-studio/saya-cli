//! Evidence binding to the candidate source digest: verify-at-use, fail closed.
//!
//! A binding pins the candidate's content digest at the moment evidence is
//! produced. Re-deriving the digest at use time and comparing is what makes
//! ANY edit invalidate the evidence: one flipped byte changes the digest.
//!
//! The binding is verify-at-use and fails closed: only an exact digest match
//! passes. Any anomaly — symlink, nested repository, bound trip, cancellation,
//! deadline, missing tree — is [`EvidenceError::Unverifiable`], never "assume
//! valid". Bindings are not persisted: a restart forgets them, and nothing
//! remembered means nothing trusted.

use std::{fmt, path::Path, time::Duration};

use saya_agent::CancellationToken;
use saya_types::{ManagedCheckoutId, Sha256Digest};

use crate::coding::worktree::VerifiedCheckout;

use super::{CandidateError, candidate_source_digest};

/// Binds evidence (a C03/C04 check receipt, an R-series review, or an accepted
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
    /// Digests the candidate now via [`candidate_source_digest`].
    pub fn bind(
        candidate: &VerifiedCheckout,
        cancellation: &CancellationToken,
        deadline: Duration,
    ) -> Result<Self, EvidenceError> {
        let source_sha256 =
            candidate_source_digest(Path::new(candidate.path().as_str()), cancellation, deadline)?;
        Ok(Self {
            candidate_checkout_id: candidate.checkout_id().clone(),
            source_sha256,
        })
    }

    pub fn candidate_checkout_id(&self) -> &ManagedCheckoutId {
        &self.candidate_checkout_id
    }

    pub fn source_sha256(&self) -> &Sha256Digest {
        &self.source_sha256
    }

    /// Re-derives the digest of the tree at `candidate_root` and compares.
    ///
    /// Mismatch is [`EvidenceError::Stale`]; any walk failure is
    /// [`EvidenceError::Unverifiable`]; only exact equality passes. Fail
    /// closed both ways.
    pub fn verify_at_use(
        &self,
        candidate_root: &Path,
        cancellation: &CancellationToken,
        deadline: Duration,
    ) -> Result<(), EvidenceError> {
        let current = candidate_source_digest(candidate_root, cancellation, deadline)?;
        if current != self.source_sha256 {
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

/// Why a binding could not be honored at use time.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum EvidenceError {
    #[error("the bound candidate tree changed after the evidence was produced")]
    Stale,
    #[error("the bound candidate tree could not be fingerprinted")]
    Unverifiable(#[from] CandidateError),
}
