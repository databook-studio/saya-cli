//! Compare one durable intent with actual files and Git; never prune or replay.
use super::{
    CleanupError, CleanupInput,
    cleanup::{can_reconcile, check_lease},
};
use crate::coding::integration::{DEFAULT_DELTA_DEADLINE, compute_delta};
use saya_agent::CancellationToken;
use saya_types::{CheckoutOperationState, ManagedCheckoutLifecycle, RepositoryMutationAction};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckoutReconciliation {
    Unchanged,
    CreateCompleted,
    RemoveCompleted,
    RequireOperator,
}

pub async fn reconcile_managed_checkout(
    input: &CleanupInput<'_>,
    cancellation: &CancellationToken,
) -> Result<CheckoutReconciliation, CleanupError> {
    check_lease(input)?;
    let owned = &input.ownership;
    if owned
        .store
        .managed_checkout_owner(owned.checkout_id)
        .await?
        .as_ref()
        != Some(owned.job_id)
    {
        return Err(super::OwnershipError::WrongOwner.into());
    }
    let record = owned
        .store
        .get_managed_checkout(owned.job_id, owned.checkout_id)
        .await?
        .ok_or(super::OwnershipError::NotFound)?;
    if record.ownership.managed_path() != &owned.service.managed_path(owned.checkout_id)? {
        return Err(super::OwnershipError::PathMismatch.into());
    }
    if !can_reconcile(record.current_lifecycle) {
        return Ok(
            if record.current_lifecycle == ManagedCheckoutLifecycle::Uncertain {
                CheckoutReconciliation::RequireOperator
            } else {
                CheckoutReconciliation::Unchanged
            },
        );
    }
    if record.latest_operation.state() != CheckoutOperationState::Intent {
        return Ok(CheckoutReconciliation::RequireOperator);
    }
    let path = Path::new(record.ownership.managed_path().as_str());
    let exists = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        _ => return Ok(CheckoutReconciliation::RequireOperator),
    };
    let action = record.latest_operation.action();
    let result = match (action, exists) {
        (RepositoryMutationAction::CreateCheckout, true) => {
            let verified = match super::ownership::verify_checkout_state(
                owned,
                cancellation,
                ManagedCheckoutLifecycle::Creating,
            )
            .await
            {
                Ok(verified) => verified,
                Err(_) => return Ok(CheckoutReconciliation::RequireOperator),
            };
            let snapshot = owned.artifacts.read_snapshot(verified.snapshot_id())?;
            if !compute_delta(&verified, &snapshot, cancellation, DEFAULT_DELTA_DEADLINE)?
                .is_empty()
            {
                return Ok(CheckoutReconciliation::RequireOperator);
            }
            CheckoutReconciliation::CreateCompleted
        }
        (RepositoryMutationAction::RemoveCheckout, false) => {
            let inventory = match owned
                .service
                .inventory_repository(owned.repository, cancellation)
                .await
            {
                Ok(inventory) => inventory,
                Err(_) => return Ok(CheckoutReconciliation::RequireOperator),
            };
            if inventory.identity().repository_id() != record.ownership.repository_id()
                || inventory.identity().common_dir_sha256() != record.ownership.common_dir_sha256()
                || inventory
                    .worktrees()
                    .iter()
                    .any(|entry| entry.path() == path)
            {
                return Ok(CheckoutReconciliation::RequireOperator);
            }
            CheckoutReconciliation::RemoveCompleted
        }
        _ => return Ok(CheckoutReconciliation::RequireOperator),
    };
    super::repair::append_outcome(
        input,
        record.latest_operation.operation_sequence(),
        action,
        CheckoutOperationState::Completed,
    )
    .await?;
    Ok(result)
}
