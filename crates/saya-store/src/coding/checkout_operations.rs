use saya_types::{
    CheckoutOperationState, CheckoutOperationV1, CodingEventKind, CodingEventV1,
    ManagedCheckoutLifecycle, RepositoryMutationAction,
};
use sqlx::{Sqlite, Transaction};

use super::{
    checkout_event::{operation_in_tx, sql_sequence},
    checkout_reads::managed_checkout_in_tx,
    checkouts::{finish, insert_operation, rollback},
    events::{CodingEventAppend, append_event_in_tx},
};
use crate::{SqliteStateStore, StoreError};

pub const MAX_CHECKOUT_OPERATION_INTENTS_PER_JOB: usize = 10_000;

impl SqliteStateStore {
    /// Records one non-create intent; it grants no permission to perform the effect.
    pub async fn record_checkout_operation_intent(
        &self,
        event: CodingEventV1,
    ) -> Result<CodingEventAppend, StoreError> {
        let operation = intent_payload(&event)?;
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
            validate_intent(&mut transaction, &event, &operation).await?;
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM coding_checkout_operations WHERE job_id=?",
            )
            .bind(event.job_id().as_str())
            .fetch_one(&mut *transaction)
            .await
            .map_err(|_| StoreError::Unavailable)?;
            if usize::try_from(count).map_err(|_| StoreError::Invalid)?
                >= MAX_CHECKOUT_OPERATION_INTENTS_PER_JOB
            {
                return Err(StoreError::LimitExceeded);
            }
            insert_operation(&mut transaction, &operation).await?;
            sqlx::query("UPDATE coding_managed_checkouts SET latest_operation_sequence=? WHERE job_id=? AND checkout_id=?")
                .bind(sql_sequence(event.sequence())?)
                .bind(event.job_id().as_str())
                .bind(operation.managed_checkout_id().as_str())
                .execute(&mut *transaction)
                .await
                .map_err(|_| StoreError::Unavailable)?;
            Ok(CodingEventAppend::Appended(event.sequence()))
        }
        .await;
        finish(transaction, result).await
    }

    /// Records the referenced result, including late outcomes after job termination.
    pub async fn record_checkout_operation_outcome(
        &self,
        event: CodingEventV1,
    ) -> Result<CodingEventAppend, StoreError> {
        let outcome = outcome_payload(&event)?;
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
            validate_outcome(&mut transaction, &event, &outcome).await?;
            let updated = sqlx::query("UPDATE coding_checkout_operations SET outcome_sequence=? WHERE job_id=? AND operation_sequence=? AND outcome_sequence IS NULL")
                .bind(sql_sequence(event.sequence())?)
                .bind(event.job_id().as_str())
                .bind(sql_sequence(outcome.operation_sequence())?)
                .execute(&mut *transaction)
                .await
                .map_err(|_| StoreError::Unavailable)?;
            if updated.rows_affected() != 1 {
                return Err(StoreError::Conflict);
            }
            Ok(CodingEventAppend::Appended(event.sequence()))
        }
        .await;
        finish(transaction, result).await
    }
}

async fn validate_intent(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &CodingEventV1,
    operation: &CheckoutOperationV1,
) -> Result<(), StoreError> {
    if operation.job_id() != event.job_id()
        || operation.operation_sequence() != event.sequence()
        || operation.action() == RepositoryMutationAction::CreateCheckout
        || operation.state() != CheckoutOperationState::Intent
        || event.invocation_id().is_some()
    {
        return Err(StoreError::Conflict);
    }
    let owned =
        managed_checkout_in_tx(transaction, event.job_id(), operation.managed_checkout_id())
            .await?
            .ok_or(StoreError::NotFound)?;
    if event.agent_id() != owned.ownership.agent_id() {
        return Err(StoreError::Conflict);
    }
    let create = operation_in_tx(
        transaction,
        event.job_id(),
        owned.ownership.created_by_sequence(),
    )
    .await?;
    if create.action() != RepositoryMutationAction::CreateCheckout
        || create.state() != CheckoutOperationState::Completed
    {
        return Err(StoreError::Conflict);
    }
    let latest = &owned.latest_operation;
    if matches!(
        latest.state(),
        CheckoutOperationState::Intent | CheckoutOperationState::Uncertain
    ) || !matches!(
        owned.current_lifecycle,
        ManagedCheckoutLifecycle::Active | ManagedCheckoutLifecycle::Retained
    ) {
        return Err(StoreError::Conflict);
    }
    let state: String = sqlx::query_scalar("SELECT state FROM coding_jobs WHERE job_id=?")
        .bind(event.job_id().as_str())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?
        .ok_or(StoreError::NotFound)?;
    match operation.action() {
        RepositoryMutationAction::IntegrateCandidate | RepositoryMutationAction::ApplyBack => {
            if state != "running" {
                return Err(StoreError::Conflict);
            }
        }
        RepositoryMutationAction::RemoveCheckout => {
            if state != "running" && !matches!(state.as_str(), "completed" | "failed" | "cancelled")
            {
                return Err(StoreError::Conflict);
            }
        }
        _ => return Err(StoreError::Invalid),
    }
    Ok(())
}

async fn validate_outcome(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &CodingEventV1,
    outcome: &CheckoutOperationV1,
) -> Result<(), StoreError> {
    if outcome.job_id() != event.job_id()
        || outcome.operation_sequence() >= event.sequence()
        || outcome.state() == CheckoutOperationState::Intent
    {
        return Err(StoreError::Conflict);
    }
    let intent = operation_in_tx(transaction, event.job_id(), outcome.operation_sequence()).await?;
    let owned = managed_checkout_in_tx(transaction, event.job_id(), outcome.managed_checkout_id())
        .await?
        .ok_or(StoreError::NotFound)?;
    if intent.managed_checkout_id() != outcome.managed_checkout_id()
        || intent.action() != outcome.action()
        || intent.state() != CheckoutOperationState::Intent
        || owned.ownership.agent_id() != event.agent_id()
        || event.invocation_id().is_some()
    {
        return Err(StoreError::Conflict);
    }
    Ok(())
}

fn intent_payload(event: &CodingEventV1) -> Result<CheckoutOperationV1, StoreError> {
    match event.event() {
        CodingEventKind::CheckoutOperationIntent { operation } => Ok(operation.as_ref().clone()),
        _ => Err(StoreError::Invalid),
    }
}

fn outcome_payload(event: &CodingEventV1) -> Result<CheckoutOperationV1, StoreError> {
    match event.event() {
        CodingEventKind::CheckoutOperationOutcome { operation } => Ok(operation.as_ref().clone()),
        _ => Err(StoreError::Invalid),
    }
}
