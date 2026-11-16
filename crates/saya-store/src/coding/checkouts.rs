use saya_types::{
    CheckoutOperationState, CheckoutOperationV1, CodingEventKind, CodingEventV1, ManagedCheckoutId,
    ManagedCheckoutLifecycle, ManagedCheckoutRecordV1, RepositoryMutationAction,
};
use sqlx::{Row, Sqlite, Transaction};

use super::{
    checkout_event::sql_sequence,
    events::{CodingEventAppend, append_event_in_tx},
};
use crate::{SqliteStateStore, StoreError};

pub const MAX_CHECKOUT_RECORDS_PER_JOB: usize = 256;

pub(crate) fn validate_limit(limit: usize) -> Result<(), StoreError> {
    if limit == 0 {
        return Err(StoreError::Invalid);
    }
    if limit > super::events::MAX_CODING_EVENT_PAGE_SIZE {
        return Err(StoreError::LimitExceeded);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedCheckoutRecord {
    /// Immutable original creation metadata, including the private path.
    pub ownership: ManagedCheckoutRecordV1,
    /// Derived from the latest operation. `Retained` prevents automatic further
    /// child writes and does not prove that integration succeeded.
    pub current_lifecycle: ManagedCheckoutLifecycle,
    pub latest_operation: CheckoutOperationV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedCheckoutPage {
    pub items: Vec<ManagedCheckoutRecord>,
    pub next_cursor: Option<ManagedCheckoutId>,
}

impl SqliteStateStore {
    /// Persists ownership and create intent together before any external effect.
    /// The returned event receipt is audit evidence, not permission to mutate Git.
    pub async fn record_checkout_create_intent(
        &self,
        event: CodingEventV1,
    ) -> Result<CodingEventAppend, StoreError> {
        let (checkout, operation) = create_payload(&event)?;
        let pool = self.pool().await?;
        let mut transaction = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        match append_event_in_tx(&mut transaction, event.clone()).await {
            Ok(CodingEventAppend::AlreadyPresent(sequence)) => {
                transaction
                    .commit()
                    .await
                    .map_err(|_| StoreError::Unavailable)?;
                return Ok(CodingEventAppend::AlreadyPresent(sequence));
            }
            Ok(CodingEventAppend::Appended(_)) => {}
            Err(error) => return rollback(transaction, Err(error)).await,
        }

        let result = async {
            validate_create(&mut transaction, &event, &checkout, &operation).await?;
            let job_id = event.job_id().as_str();
            let checkout_id = checkout.managed_checkout_id().as_str();
            let sequence = sql_sequence(event.sequence())?;
            sqlx::query("INSERT INTO coding_managed_checkouts(job_id, checkout_id, created_by_sequence, latest_operation_sequence) VALUES (?, ?, ?, ?)")
                .bind(job_id)
                .bind(checkout_id)
                .bind(sequence)
                .bind(sequence)
                .execute(&mut *transaction)
                .await
                .map_err(|_| StoreError::Unavailable)?;
            insert_operation(&mut transaction, &operation).await?;
            Ok(CodingEventAppend::Appended(event.sequence()))
        }
        .await;
        finish(transaction, result).await
    }
}

async fn validate_create(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &CodingEventV1,
    checkout: &ManagedCheckoutRecordV1,
    operation: &CheckoutOperationV1,
) -> Result<(), StoreError> {
    let row =
        sqlx::query("SELECT repository_id, snapshot_id, state FROM coding_jobs WHERE job_id=?")
            .bind(event.job_id().as_str())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| StoreError::Unavailable)?
            .ok_or(StoreError::NotFound)?;
    let repository: String = row
        .try_get("repository_id")
        .map_err(|_| StoreError::Invalid)?;
    let snapshot: String = row
        .try_get("snapshot_id")
        .map_err(|_| StoreError::Invalid)?;
    let state: String = row.try_get("state").map_err(|_| StoreError::Invalid)?;
    if state != "running"
        || repository != checkout.repository_id().as_str()
        || snapshot != checkout.snapshot_id().as_str()
        || checkout.job_id() != event.job_id()
        || operation.job_id() != event.job_id()
        || operation.managed_checkout_id() != checkout.managed_checkout_id()
        || operation.action() != RepositoryMutationAction::CreateCheckout
        || operation.state() != CheckoutOperationState::Intent
        || checkout.created_by_sequence() != event.sequence()
        || operation.operation_sequence() != event.sequence()
        || event.agent_id() != checkout.agent_id()
        || event.invocation_id().is_some()
    {
        return Err(StoreError::Conflict);
    }
    if let Some(agent_id) = checkout.agent_id() {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM coding_children WHERE job_id=? AND agent_id=?)",
        )
        .bind(event.job_id().as_str())
        .bind(agent_id.as_str())
        .fetch_one(&mut **transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?;
        if !exists {
            return Err(StoreError::NotFound);
        }
    }
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM coding_managed_checkouts WHERE job_id=? AND checkout_id=?)",
    )
    .bind(event.job_id().as_str())
    .bind(checkout.managed_checkout_id().as_str())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if exists {
        return Err(StoreError::Conflict);
    }
    // The managed path derives from the checkout id alone, so an id already
    // owned by ANOTHER job must never be claimed twice.
    let elsewhere: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM coding_managed_checkouts WHERE checkout_id=? AND job_id!=?)",
    )
    .bind(checkout.managed_checkout_id().as_str())
    .bind(event.job_id().as_str())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if elsewhere {
        return Err(StoreError::Conflict);
    }
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM coding_managed_checkouts WHERE job_id=?")
            .bind(event.job_id().as_str())
            .fetch_one(&mut **transaction)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    if usize::try_from(count).map_err(|_| StoreError::Invalid)? >= MAX_CHECKOUT_RECORDS_PER_JOB {
        return Err(StoreError::LimitExceeded);
    }
    let operations: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM coding_checkout_operations WHERE job_id=?")
            .bind(event.job_id().as_str())
            .fetch_one(&mut **transaction)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    if usize::try_from(operations).map_err(|_| StoreError::Invalid)?
        >= super::checkout_operations::MAX_CHECKOUT_OPERATION_INTENTS_PER_JOB
    {
        return Err(StoreError::LimitExceeded);
    }
    Ok(())
}

pub(super) async fn insert_operation(
    transaction: &mut Transaction<'_, Sqlite>,
    operation: &CheckoutOperationV1,
) -> Result<(), StoreError> {
    sqlx::query("INSERT INTO coding_checkout_operations(job_id, operation_sequence, checkout_id, action, outcome_sequence) VALUES (?, ?, ?, ?, NULL)")
        .bind(operation.job_id().as_str())
        .bind(sql_sequence(operation.operation_sequence())?)
        .bind(operation.managed_checkout_id().as_str())
        .bind(action_name(operation.action())?)
        .execute(&mut **transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

fn create_payload(
    event: &CodingEventV1,
) -> Result<(ManagedCheckoutRecordV1, CheckoutOperationV1), StoreError> {
    match event.event() {
        CodingEventKind::CheckoutCreateIntent {
            checkout,
            operation,
        } => Ok((checkout.as_ref().clone(), operation.as_ref().clone())),
        _ => Err(StoreError::Invalid),
    }
}

pub(super) fn action_name(action: RepositoryMutationAction) -> Result<&'static str, StoreError> {
    Ok(match action {
        RepositoryMutationAction::CreateCheckout => "create_checkout",
        RepositoryMutationAction::RemoveCheckout => "remove_checkout",
        RepositoryMutationAction::IntegrateCandidate => "integrate_candidate",
        RepositoryMutationAction::ApplyBack => "apply_back",
        _ => return Err(StoreError::Invalid),
    })
}

pub(super) async fn finish<T>(
    transaction: Transaction<'_, Sqlite>,
    result: Result<T, StoreError>,
) -> Result<T, StoreError> {
    match result {
        Ok(value) => {
            transaction
                .commit()
                .await
                .map_err(|_| StoreError::Unavailable)?;
            Ok(value)
        }
        Err(error) => rollback(transaction, Err(error)).await,
    }
}

pub(super) async fn rollback<T>(
    transaction: Transaction<'_, Sqlite>,
    result: Result<T, StoreError>,
) -> Result<T, StoreError> {
    if transaction.rollback().await.is_err() {
        return Err(StoreError::Unavailable);
    }
    result
}
