//! Renewed checkout authority and one captured-state identity for apply-back and recovery.

use saya_agent::CancellationToken;

use super::transaction::{ApplyBackError, ApplyBackInput};
use crate::coding::worktree::{OwnershipInput, VerifiedCheckout, verify_owned_checkout};

pub(super) async fn verify_apply_back_checkouts(
    input: &ApplyBackInput<'_>,
    cancellation: &CancellationToken,
) -> Result<(), ApplyBackError> {
    if input.plan.digest() != input.baseline.plan_manifest_sha256() {
        return Err(ApplyBackError::SnapshotMismatch);
    }
    for verified in std::iter::once(input.candidate)
        .chain(input.accepted.iter().map(|accepted| accepted.verified()))
    {
        reverify(input, verified, cancellation).await?;
    }
    Ok(())
}

async fn reverify(
    input: &ApplyBackInput<'_>,
    verified: &VerifiedCheckout,
    cancellation: &CancellationToken,
) -> Result<(), ApplyBackError> {
    let fresh = verify_owned_checkout(
        &OwnershipInput {
            store: input.store,
            service: input.service,
            repository: input.repository,
            artifacts: input.artifacts,
            job_id: input.job_id,
            checkout_id: verified.checkout_id(),
        },
        cancellation,
    )
    .await?;
    if fresh.path() != verified.path()
        || fresh.snapshot_sha256() != verified.snapshot_sha256()
        || fresh.snapshot_id() != verified.snapshot_id()
        || fresh.plan_sha256() != verified.plan_sha256()
        || fresh.base_commit() != verified.base_commit()
        || fresh.agent_id() != verified.agent_id()
    {
        return Err(ApplyBackError::StaleVerification);
    }
    if fresh.snapshot_id() != input.baseline.snapshot_id()
        || fresh.snapshot_sha256() != input.baseline.captured_manifest_sha256()
        || fresh.plan_sha256() != input.plan.digest()
    {
        return Err(ApplyBackError::SnapshotMismatch);
    }
    Ok(())
}
