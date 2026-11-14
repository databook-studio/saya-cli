use std::num::NonZeroU64;

use saya_types::{
    CheckoutOperationState, CheckoutOperationV1, CodingEventKind, CodingEventV1, JobId,
    ManagedCheckoutId, ManagedCheckoutRecordV1, RepositoryMutationAction,
};
use sqlx::{Row, Sqlite, Transaction};

use super::events::event_error;
use crate::StoreError;

pub(crate) async fn event_in_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    job_id: &JobId,
    sequence: NonZeroU64,
) -> Result<CodingEventV1, StoreError> {
    let row = sqlx::query(
        "SELECT job_id, sequence, event_json FROM coding_events WHERE job_id=? AND sequence=?",
    )
    .bind(job_id.as_str())
    .bind(sql_sequence(sequence)?)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?
    .ok_or(StoreError::Invalid)?;
    let indexed_job = JobId::parse(
        &row.try_get::<String, _>("job_id")
            .map_err(|_| StoreError::Invalid)?,
    )
    .map_err(|_| StoreError::Invalid)?;
    let indexed_sequence =
        positive_sequence(row.try_get("sequence").map_err(|_| StoreError::Invalid)?)?;
    let json: String = row.try_get("event_json").map_err(|_| StoreError::Invalid)?;
    let event = CodingEventV1::from_json_bytes(json.as_bytes()).map_err(event_error)?;
    if &indexed_job != job_id
        || indexed_sequence != sequence
        || event.job_id() != &indexed_job
        || event.sequence() != indexed_sequence
    {
        return Err(StoreError::Invalid);
    }
    Ok(event)
}

pub(crate) fn positive_sequence(value: i64) -> Result<NonZeroU64, StoreError> {
    u64::try_from(value)
        .ok()
        .and_then(NonZeroU64::new)
        .ok_or(StoreError::Invalid)
}

pub(crate) fn sql_sequence(sequence: NonZeroU64) -> Result<i64, StoreError> {
    i64::try_from(sequence.get()).map_err(|_| StoreError::LimitExceeded)
}

pub(crate) async fn operation_in_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    job_id: &JobId,
    sequence: NonZeroU64,
) -> Result<CheckoutOperationV1, StoreError> {
    let row = sqlx::query("SELECT checkout_id, action, outcome_sequence FROM coding_checkout_operations WHERE job_id=? AND operation_sequence=?")
        .bind(job_id.as_str())
        .bind(sql_sequence(sequence)?)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?
        .ok_or(StoreError::Invalid)?;
    let checkout_id = ManagedCheckoutId::parse(
        &row.try_get::<String, _>("checkout_id")
            .map_err(|_| StoreError::Invalid)?,
    )
    .map_err(|_| StoreError::Invalid)?;
    let ownership_row = sqlx::query(
        "SELECT created_by_sequence FROM coding_managed_checkouts WHERE job_id=? AND checkout_id=?",
    )
    .bind(job_id.as_str())
    .bind(checkout_id.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?
    .ok_or(StoreError::Invalid)?;
    let created_by = positive_sequence(
        ownership_row
            .try_get("created_by_sequence")
            .map_err(|_| StoreError::Invalid)?,
    )?;
    let (ownership, _) = creation_in_tx(transaction, job_id, &checkout_id, created_by).await?;
    let action = parse_action(
        &row.try_get::<String, _>("action")
            .map_err(|_| StoreError::Invalid)?,
    )?;
    let intent_event = event_in_tx(transaction, job_id, sequence).await?;
    let operation = match intent_event.event() {
        CodingEventKind::CheckoutCreateIntent { operation, .. }
        | CodingEventKind::CheckoutOperationIntent { operation } => operation.as_ref().clone(),
        _ => return Err(StoreError::Invalid),
    };
    if intent_event.agent_id() != ownership.agent_id()
        || operation.job_id() != job_id
        || operation.operation_sequence() != sequence
        || operation.managed_checkout_id() != &checkout_id
        || operation.action() != action
        || operation.state() != CheckoutOperationState::Intent
    {
        return Err(StoreError::Invalid);
    }
    let outcome_sequence: Option<i64> = row
        .try_get("outcome_sequence")
        .map_err(|_| StoreError::Invalid)?;
    let Some(outcome_sequence) = outcome_sequence else {
        return Ok(operation);
    };
    let outcome_sequence = positive_sequence(outcome_sequence)?;
    if outcome_sequence <= sequence {
        return Err(StoreError::Invalid);
    }
    let outcome_event = event_in_tx(transaction, job_id, outcome_sequence).await?;
    let outcome = match outcome_event.event() {
        CodingEventKind::CheckoutOperationOutcome { operation } => operation.as_ref().clone(),
        _ => return Err(StoreError::Invalid),
    };
    if outcome_event.agent_id() != ownership.agent_id()
        || outcome.job_id() != job_id
        || outcome.operation_sequence() != sequence
        || outcome.managed_checkout_id() != &checkout_id
        || outcome.action() != action
        || outcome.state() == CheckoutOperationState::Intent
    {
        return Err(StoreError::Invalid);
    }
    Ok(outcome)
}

pub(crate) async fn creation_in_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    job_id: &JobId,
    checkout_id: &ManagedCheckoutId,
    created_by: NonZeroU64,
) -> Result<(ManagedCheckoutRecordV1, CheckoutOperationV1), StoreError> {
    let event = event_in_tx(transaction, job_id, created_by).await?;
    let (ownership, operation) = match event.event() {
        CodingEventKind::CheckoutCreateIntent {
            checkout,
            operation,
        } => (checkout.as_ref().clone(), operation.as_ref().clone()),
        _ => return Err(StoreError::Invalid),
    };
    if ownership.job_id() != job_id
        || ownership.managed_checkout_id() != checkout_id
        || ownership.created_by_sequence() != created_by
        || event.agent_id() != ownership.agent_id()
        || event.invocation_id().is_some()
        || operation.job_id() != job_id
        || operation.operation_sequence() != created_by
        || operation.managed_checkout_id() != checkout_id
        || operation.action() != RepositoryMutationAction::CreateCheckout
        || operation.state() != CheckoutOperationState::Intent
    {
        return Err(StoreError::Invalid);
    }
    Ok((ownership, operation))
}

fn parse_action(value: &str) -> Result<RepositoryMutationAction, StoreError> {
    match value {
        "create_checkout" => Ok(RepositoryMutationAction::CreateCheckout),
        "remove_checkout" => Ok(RepositoryMutationAction::RemoveCheckout),
        "integrate_candidate" => Ok(RepositoryMutationAction::IntegrateCandidate),
        "apply_back" => Ok(RepositoryMutationAction::ApplyBack),
        _ => Err(StoreError::Invalid),
    }
}
