//! Orchestrates `integrate_candidate`: the admission preflight, the overlap
//! proof, the intent append, the serial apply, the source digest, and the
//! outcome record.
//!
//! Journal discipline: the intent is appended only after every refusal that
//! can be decided from caller input or durable state has been ruled out, so a
//! refused run never journals and never poisons the ledger owner. Failures
//! after the intent record an outcome: a failed apply records `Failed`, and a
//! digest walk that fails after a full apply records `Uncertain` because the
//! candidate's final content cannot be proven.
//!
//! Crash semantics: a crash between intent and outcome leaves one open
//! `IntegrateCandidate`/`Intent` operation on the candidate. That checkout id
//! is then terminal — the store refuses every further operation on it — and
//! the safe recovery is discarding the partially-applied candidate and
//! rebuilding with a FRESH candidate checkout id through the normal create
//! path. Never resume a partially-applied candidate. Retries never re-append
//! blindly: the sequencer treats an `AlreadyPresent` dedup as a poisoned
//! owner, and the open intent surfaces at preflight as
//! `IntegrateError::CandidateBusy`. No durable digest is stored: the returned
//! `IntegratedCandidate::source_sha256` is the only witness; durable digest
//! evidence is deferred to later packets.

use std::{fmt, num::NonZeroU64, path::Path, time::Duration};

use saya_agent::CancellationToken;
use saya_store::{SqliteStateStore, StoreError};
use saya_types::{AgentId, CheckoutOperationState, JobId, Sha256Digest};

use crate::coding::budget::{JobEventSequencer, SequencerError};
use crate::coding::snapshot::CapturedSnapshot;
use crate::coding::worktree::VerifiedCheckout;

use super::apply::{AcceptedDelta, ApplyError, OverlapConflict, apply_delta, detect_overlaps};
use super::candidate::{CandidateError, CandidateIdentity, candidate_source_digest};
use super::integrate_events::{
    append_integrate_intent, preflight_integrate_admission, record_integrate_outcome,
};

/// Borrowed handles for one `integrate_candidate` run; the shape mirrors the
/// create flow's input.
pub struct IntegrateInput<'a> {
    pub store: &'a SqliteStateStore,
    pub sequencer: &'a JobEventSequencer,
    pub job_id: &'a JobId,
    /// Must equal the candidate's recorded ownership agent; the store re-checks.
    pub agent_id: Option<&'a AgentId>,
    /// The candidate checkout, verified at use time.
    pub candidate: &'a VerifiedCheckout,
    /// The published baseline snapshot the candidate was materialized from.
    pub baseline: &'a CapturedSnapshot,
    /// Accepted, receipt-proven deltas, in child-result order.
    pub accepted: &'a [AcceptedDelta<'a>],
}

/// Why an integration did not complete.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IntegrateError {
    #[error("the local state store failed")]
    Store(#[from] StoreError),
    #[error("the job event ledger refused the append")]
    Sequencer(#[from] SequencerError),
    #[error("a delta could not be applied")]
    Apply(#[from] ApplyError),
    #[error("the candidate could not be fingerprinted")]
    Digest(#[from] CandidateError),
    #[error("the accepted deltas overlap")]
    Overlap(OverlapConflict),
    #[error("the job is not running")]
    JobNotRunning,
    #[error("the job reached its checkout operation limit")]
    OperationCapReached,
    #[error("the candidate checkout is not active")]
    CandidateNotActive,
    #[error("an operation is already open on the candidate")]
    CandidateBusy,
    #[error("the agent does not match the candidate ownership")]
    AgentMismatch,
    #[error("the integration was cancelled")]
    Cancelled,
}

/// The result of one successful `integrate_candidate` run: the candidate's
/// identity, its content digest, how many deltas were applied, and the journal
/// sequence of the completed operation.
///
/// The [`fmt::Debug`] output carries the checkout id and the applied count
/// only — never a digest and never a filesystem path.
pub struct IntegratedCandidate {
    identity: CandidateIdentity,
    source_sha256: Sha256Digest,
    applied: usize,
    operation_sequence: NonZeroU64,
}

impl IntegratedCandidate {
    pub fn identity(&self) -> &CandidateIdentity {
        &self.identity
    }

    pub fn source_sha256(&self) -> &Sha256Digest {
        &self.source_sha256
    }

    pub fn applied(&self) -> usize {
        self.applied
    }

    pub fn operation_sequence(&self) -> NonZeroU64 {
        self.operation_sequence
    }
}

impl fmt::Debug for IntegratedCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IntegratedCandidate")
            .field("checkout_id", &self.identity.checkout_id().as_str())
            .field("applied", &self.applied)
            .finish()
    }
}

/// Integrates the accepted deltas into the candidate checkout, journaling the
/// operation around the work: preflight admission, overlap proof, intent
/// append, serial apply, source digest, then the outcome record. Failures
/// before the intent append journal nothing; failures after it record an
/// outcome (`Failed` for an apply refusal, `Uncertain` for a digest walk that
/// fails after a full apply), and a journaling failure on the outcome never
/// masks the primary error.
pub async fn integrate_candidate(
    input: &IntegrateInput<'_>,
    cancellation: &CancellationToken,
    deadline: Duration,
) -> Result<IntegratedCandidate, IntegrateError> {
    if cancellation.is_cancelled() {
        return Err(IntegrateError::Cancelled);
    }

    preflight_integrate_admission(input).await?;

    // A conflict is a caller-input property, pure to detect, and must never
    // journal: recording it would burn a sequence and strand the candidate
    // under the open-operation unique index until a manual outcome.
    if let Err(conflict) = detect_overlaps(input.accepted) {
        return Err(IntegrateError::Overlap(conflict));
    }

    if cancellation.is_cancelled() {
        return Err(IntegrateError::Cancelled);
    }

    let operation_sequence = append_integrate_intent(input).await?;

    let candidate_root = Path::new(input.candidate.path().as_str());
    let post_intent = (|| -> Result<(Sha256Digest, usize), IntegrateError> {
        let mut applied = 0usize;
        for delta in input.accepted {
            apply_delta(candidate_root, delta, input.baseline, cancellation)?;
            applied += 1;
        }
        let source_sha256 = candidate_source_digest(candidate_root, cancellation, deadline)?;
        Ok((source_sha256, applied))
    })();

    match post_intent {
        Ok((source_sha256, applied)) => {
            record_integrate_outcome(input, operation_sequence, CheckoutOperationState::Completed)
                .await?;
            Ok(IntegratedCandidate {
                identity: CandidateIdentity::from_verified(input.job_id, input.candidate),
                source_sha256,
                applied,
                operation_sequence,
            })
        }
        Err(IntegrateError::Apply(error)) => {
            // The run result is known-incomplete and every guarded write's
            // local effect is known, so the outcome is `Failed`, not
            // `Uncertain`; a journaling failure never masks the primary error.
            let _ =
                record_integrate_outcome(input, operation_sequence, CheckoutOperationState::Failed)
                    .await;
            Err(IntegrateError::Apply(error))
        }
        Err(IntegrateError::Digest(error)) => {
            // Every delta is fully applied, but the candidate's final content
            // cannot be proven: the outcome is `Uncertain`.
            let _ = record_integrate_outcome(
                input,
                operation_sequence,
                CheckoutOperationState::Uncertain,
            )
            .await;
            Err(IntegrateError::Digest(error))
        }
        Err(other) => Err(other),
    }
}
