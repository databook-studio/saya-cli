use std::num::NonZeroU64;

use saya_types::{
    CheckoutOperationState, CheckoutOperationV1, JobId, ManagedCheckoutId,
    ManagedCheckoutLifecycle, RepositoryMutationAction,
};
use sqlx::{Row, Sqlite, Transaction};

use super::{
    checkout_event::{operation_in_tx, positive_sequence, sql_sequence},
    checkouts::{ManagedCheckoutPage, ManagedCheckoutRecord, validate_limit},
};
use crate::{SqliteStateStore, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckoutOperationPage {
    pub items: Vec<CheckoutOperationV1>,
    pub next_cursor: Option<NonZeroU64>,
}

impl SqliteStateStore {
    /// Returns immutable creation metadata with lifecycle derived from its latest operation.
    pub async fn get_managed_checkout(
        &self,
        job_id: &JobId,
        checkout_id: &ManagedCheckoutId,
    ) -> Result<Option<ManagedCheckoutRecord>, StoreError> {
        let pool = self.pool().await?;
        let mut transaction = pool.begin().await.map_err(|_| StoreError::Unavailable)?;
        let record = managed_checkout_in_tx(&mut transaction, job_id, checkout_id).await?;
        transaction
            .commit()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        Ok(record)
    }

    /// Lists ownership records in checkout-ID order with a bounded keyset page.
    pub async fn list_managed_checkouts(
        &self,
        job_id: &JobId,
        after: Option<&ManagedCheckoutId>,
        limit: usize,
    ) -> Result<ManagedCheckoutPage, StoreError> {
        validate_limit(limit)?;
        let pool = self.pool().await?;
        let mut transaction = pool.begin().await.map_err(|_| StoreError::Unavailable)?;
        let rows = sqlx::query("SELECT checkout_id FROM coding_managed_checkouts WHERE job_id=? AND (? IS NULL OR checkout_id > ?) ORDER BY checkout_id LIMIT ?")
            .bind(job_id.as_str())
            .bind(after.map(ManagedCheckoutId::as_str))
            .bind(after.map(ManagedCheckoutId::as_str))
            .bind(i64::try_from(limit + 1).map_err(|_| StoreError::LimitExceeded)?)
            .fetch_all(&mut *transaction)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let more = rows.len() > limit;
        let mut items = Vec::with_capacity(rows.len().min(limit));
        for row in rows.into_iter().take(limit) {
            let id = ManagedCheckoutId::parse(
                &row.try_get::<String, _>("checkout_id")
                    .map_err(|_| StoreError::Invalid)?,
            )
            .map_err(|_| StoreError::Invalid)?;
            items.push(
                managed_checkout_in_tx(&mut transaction, job_id, &id)
                    .await?
                    .ok_or(StoreError::Invalid)?,
            );
        }
        let next_cursor = more
            .then(|| {
                items
                    .last()
                    .map(|item| item.ownership.managed_checkout_id().clone())
            })
            .flatten();
        transaction
            .commit()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        Ok(ManagedCheckoutPage { items, next_cursor })
    }

    /// Returns an operation only when both its job and checkout identities match.
    pub async fn get_checkout_operation(
        &self,
        job_id: &JobId,
        checkout_id: &ManagedCheckoutId,
        operation_sequence: NonZeroU64,
    ) -> Result<Option<CheckoutOperationV1>, StoreError> {
        let pool = self.pool().await?;
        let mut transaction = pool.begin().await.map_err(|_| StoreError::Unavailable)?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM coding_checkout_operations WHERE job_id=? AND checkout_id=? AND operation_sequence=?)",
        )
        .bind(job_id.as_str())
        .bind(checkout_id.as_str())
        .bind(sql_sequence(operation_sequence)?)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?;
        let operation = if exists {
            Some(operation_in_tx(&mut transaction, job_id, operation_sequence).await?)
        } else {
            None
        };
        transaction
            .commit()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        Ok(operation)
    }

    /// Lists one checkout's operations in original-intent sequence order.
    pub async fn list_checkout_operations(
        &self,
        job_id: &JobId,
        checkout_id: &ManagedCheckoutId,
        after: Option<NonZeroU64>,
        limit: usize,
    ) -> Result<CheckoutOperationPage, StoreError> {
        validate_limit(limit)?;
        let after = after.map(sql_sequence).transpose()?.unwrap_or(0);
        let pool = self.pool().await?;
        let mut transaction = pool.begin().await.map_err(|_| StoreError::Unavailable)?;
        let rows = sqlx::query("SELECT operation_sequence FROM coding_checkout_operations WHERE job_id=? AND checkout_id=? AND operation_sequence>? ORDER BY operation_sequence LIMIT ?")
            .bind(job_id.as_str())
            .bind(checkout_id.as_str())
            .bind(after)
            .bind(i64::try_from(limit + 1).map_err(|_| StoreError::LimitExceeded)?)
            .fetch_all(&mut *transaction)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let more = rows.len() > limit;
        let mut items = Vec::with_capacity(rows.len().min(limit));
        for row in rows.into_iter().take(limit) {
            let sequence = positive_sequence(
                row.try_get("operation_sequence")
                    .map_err(|_| StoreError::Invalid)?,
            )?;
            items.push(operation_in_tx(&mut transaction, job_id, sequence).await?);
        }
        let next_cursor = more
            .then(|| items.last().map(CheckoutOperationV1::operation_sequence))
            .flatten();
        transaction
            .commit()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        Ok(CheckoutOperationPage { items, next_cursor })
    }
}

pub(crate) async fn managed_checkout_in_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    job_id: &JobId,
    checkout_id: &ManagedCheckoutId,
) -> Result<Option<ManagedCheckoutRecord>, StoreError> {
    let row = sqlx::query("SELECT created_by_sequence, latest_operation_sequence FROM coding_managed_checkouts WHERE job_id=? AND checkout_id=?")
        .bind(job_id.as_str())
        .bind(checkout_id.as_str())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let Some(row) = row else { return Ok(None) };
    let created = positive_sequence(
        row.try_get("created_by_sequence")
            .map_err(|_| StoreError::Invalid)?,
    )?;
    let latest = positive_sequence(
        row.try_get("latest_operation_sequence")
            .map_err(|_| StoreError::Invalid)?,
    )?;
    let (ownership, _) =
        super::checkout_event::creation_in_tx(transaction, job_id, checkout_id, created).await?;
    if latest < created {
        return Err(StoreError::Invalid);
    }
    let max_operation: i64 = sqlx::query_scalar(
        "SELECT MAX(operation_sequence) FROM coding_checkout_operations WHERE job_id=? AND checkout_id=?",
    )
    .bind(job_id.as_str())
    .bind(checkout_id.as_str())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if positive_sequence(max_operation)? != latest {
        return Err(StoreError::Invalid);
    }
    let latest_operation = operation_in_tx(transaction, job_id, latest).await?;
    if latest_operation.managed_checkout_id() != checkout_id {
        return Err(StoreError::Invalid);
    }
    let current_lifecycle = lifecycle(latest_operation.action(), latest_operation.state());
    Ok(Some(ManagedCheckoutRecord {
        ownership,
        current_lifecycle,
        latest_operation,
    }))
}

pub(crate) fn lifecycle(
    action: RepositoryMutationAction,
    state: CheckoutOperationState,
) -> ManagedCheckoutLifecycle {
    match (action, state) {
        (RepositoryMutationAction::CreateCheckout, CheckoutOperationState::Intent) => {
            ManagedCheckoutLifecycle::Creating
        }
        (RepositoryMutationAction::CreateCheckout, CheckoutOperationState::Completed) => {
            ManagedCheckoutLifecycle::Active
        }
        (RepositoryMutationAction::CreateCheckout, CheckoutOperationState::Failed) => {
            ManagedCheckoutLifecycle::Failed
        }
        (RepositoryMutationAction::RemoveCheckout, CheckoutOperationState::Intent) => {
            ManagedCheckoutLifecycle::Removing
        }
        (RepositoryMutationAction::RemoveCheckout, CheckoutOperationState::Completed) => {
            ManagedCheckoutLifecycle::Removed
        }
        (RepositoryMutationAction::RemoveCheckout, CheckoutOperationState::Failed) => {
            ManagedCheckoutLifecycle::Retained
        }
        (_, CheckoutOperationState::Uncertain) => ManagedCheckoutLifecycle::Uncertain,
        (RepositoryMutationAction::IntegrateCandidate | RepositoryMutationAction::ApplyBack, _) => {
            ManagedCheckoutLifecycle::Retained
        }
        _ => ManagedCheckoutLifecycle::Uncertain,
    }
}
