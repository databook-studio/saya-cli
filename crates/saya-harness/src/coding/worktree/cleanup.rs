//! Conservative disposal: only unchanged, recoverable, quiescent owned checkouts.
use super::{OwnershipError, OwnershipInput, verify_owned_checkout};
use crate::coding::{
    budget::{JobEventSequencer, JobQuiescence, SequencerError},
    git::GitError,
    integration::{DEFAULT_DELTA_DEADLINE, DeltaError, compute_delta},
    snapshot::SnapshotError,
};
use saya_agent::CancellationToken;
use saya_store::StoreError;
use saya_types::{CheckoutOperationState, ManagedCheckoutLifecycle, RepositoryMutationAction};

pub struct CleanupInput<'a> {
    pub ownership: OwnershipInput<'a>,
    pub sequencer: &'a JobEventSequencer,
    /// Held for the complete read/effect/journal operation; never a caller boolean.
    pub quiescence: &'a JobQuiescence,
}

#[derive(Debug, thiserror::Error)]
pub enum CleanupError {
    #[error(transparent)]
    Ownership(#[from] OwnershipError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    #[error(transparent)]
    Delta(#[from] DeltaError),
    #[error(transparent)]
    Sequencer(#[from] SequencerError),
    #[error("checkout owner is not quiescent for this job")]
    ForeignLease,
    #[error("retained snapshot objects are unavailable or do not verify")]
    ArtifactUnavailable,
    #[error("checkout contains work that must be retained")]
    RetainWork,
    #[error("checkout reconciliation requires operator inspection")]
    RequireOperator,
}

/// Removes an unchanged managed checkout only. Captured baseline objects and
/// the durable plan remain recoverable. No force/remove-all/prune is used.
/// A failed remove leaves its outcome durably retained or its intent recoverable.
pub async fn cleanup_managed_checkout(
    input: &CleanupInput<'_>,
    cancellation: &CancellationToken,
) -> Result<(), CleanupError> {
    check_lease(input)?;
    let owned = verify_owned_checkout(&input.ownership, cancellation).await?;
    let snapshot = input
        .ownership
        .artifacts
        .read_snapshot(owned.snapshot_id())?;
    super::retention::verify_recovery_objects(input.ownership.artifacts, &snapshot, cancellation)?;
    let delta = compute_delta(&owned, &snapshot, cancellation, DEFAULT_DELTA_DEADLINE)?;
    if !delta.is_empty() {
        return Err(CleanupError::RetainWork);
    }
    let worktree = input
        .ownership
        .service
        .adopt_worktree(
            input.ownership.repository,
            owned.base_commit(),
            owned.checkout_id(),
            cancellation,
        )
        .await?;
    // No verified reader, delta, or snapshot handle survives into disposal.
    drop((owned, snapshot, delta));
    let sequence =
        super::repair::append_intent(input, RepositoryMutationAction::RemoveCheckout).await?;
    let removed = input
        .ownership
        .service
        .remove_worktree(&worktree, cancellation)
        .await;
    let (state, error) = match removed {
        Ok(()) => (CheckoutOperationState::Completed, None),
        Err(error @ GitError::MutationOutcomeUnknown { .. }) => return Err(error.into()),
        Err(error) => (CheckoutOperationState::Failed, Some(error)),
    };
    super::repair::append_outcome(
        input,
        sequence,
        RepositoryMutationAction::RemoveCheckout,
        state,
    )
    .await?;
    error.map_or(Ok(()), |error| Err(error.into()))
}

pub(super) fn check_lease(input: &CleanupInput<'_>) -> Result<(), CleanupError> {
    if input.quiescence.job_id() != input.ownership.job_id {
        return Err(CleanupError::ForeignLease);
    }
    Ok(())
}

pub(super) fn can_reconcile(state: ManagedCheckoutLifecycle) -> bool {
    matches!(
        state,
        ManagedCheckoutLifecycle::Creating | ManagedCheckoutLifecycle::Removing
    )
}
