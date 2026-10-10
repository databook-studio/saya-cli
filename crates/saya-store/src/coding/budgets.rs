use std::num::NonZeroU64;

use saya_types::{
    BudgetReservationV1, BudgetSettlementV1, CodingEventKind, CodingEventV1, JobState,
};
use sqlx::{Row, Sqlite, Transaction};

use super::{
    budget_records::matching_keys,
    events::{CodingEventAppend, append_event_in_tx},
    records::state,
};
use crate::{SqliteStateStore, StoreError};

const MAX_UNSETTLED_RESERVATIONS: i64 = 1_024;
const MAX_RESERVATIONS: i64 = 10_000;

impl SqliteStateStore {
    pub async fn reserve_coding_budget(
        &self,
        event: CodingEventV1,
    ) -> Result<CodingEventAppend, StoreError> {
        let CodingEventKind::BudgetReserved { reservation } = event.event() else {
            return Err(StoreError::Invalid);
        };
        let reservation = reservation.clone();
        let pool = self.pool().await?;
        let mut transaction = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let outcome = match append_event_in_tx(&mut transaction, event).await {
            Ok(CodingEventAppend::AlreadyPresent(sequence)) => {
                Ok(CodingEventAppend::AlreadyPresent(sequence))
            }
            Ok(CodingEventAppend::Appended(sequence)) => {
                admit_reservation(&mut transaction, &reservation)
                    .await
                    .map(|()| CodingEventAppend::Appended(sequence))
            }
            Err(error) => Err(error),
        };
        finish(transaction, outcome).await
    }

    pub async fn settle_coding_budget(
        &self,
        event: CodingEventV1,
    ) -> Result<CodingEventAppend, StoreError> {
        let CodingEventKind::BudgetSettled { settlement } = event.event() else {
            return Err(StoreError::Invalid);
        };
        let settlement = settlement.clone();
        let pool = self.pool().await?;
        let mut transaction = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let outcome = match append_event_in_tx(&mut transaction, event).await {
            Ok(CodingEventAppend::AlreadyPresent(sequence)) => {
                Ok(CodingEventAppend::AlreadyPresent(sequence))
            }
            Ok(CodingEventAppend::Appended(sequence)) => {
                settle_reservation(&mut transaction, &settlement)
                    .await
                    .map(|()| CodingEventAppend::Appended(sequence))
            }
            Err(error) => Err(error),
        };
        finish(transaction, outcome).await
    }
}

async fn finish(
    transaction: Transaction<'_, Sqlite>,
    outcome: Result<CodingEventAppend, StoreError>,
) -> Result<CodingEventAppend, StoreError> {
    match outcome {
        Ok(append) => {
            transaction
                .commit()
                .await
                .map_err(|_| StoreError::Unavailable)?;
            Ok(append)
        }
        Err(error) => {
            transaction
                .rollback()
                .await
                .map_err(|_| StoreError::Unavailable)?;
            Err(error)
        }
    }
}

pub(super) async fn admit_reservation(
    transaction: &mut Transaction<'_, Sqlite>,
    reservation: &BudgetReservationV1,
) -> Result<(), StoreError> {
    let job_state: Option<String> =
        sqlx::query_scalar("SELECT state FROM coding_jobs WHERE job_id=?")
            .bind(reservation.job_id().as_str())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    if job_state.as_deref().map(state).transpose()? != Some(JobState::Running) {
        return Err(StoreError::Conflict);
    }
    match (reservation.agent_id(), reservation.invocation_id()) {
        (None, None) => {}
        (Some(agent_id), Some(invocation_id)) => {
            let row = sqlx::query("SELECT current_invocation_id, state FROM coding_children WHERE job_id=? AND agent_id=?")
                .bind(reservation.job_id().as_str())
                .bind(agent_id.as_str())
                .fetch_optional(&mut **transaction)
                .await
                .map_err(|_| StoreError::Unavailable)?
                .ok_or(StoreError::NotFound)?;
            let current: String = row
                .try_get("current_invocation_id")
                .map_err(|_| StoreError::Invalid)?;
            let child_state: String = row.try_get("state").map_err(|_| StoreError::Invalid)?;
            if current != invocation_id.as_str()
                || !matches!(state(&child_state)?, JobState::Queued | JobState::Running)
            {
                return Err(StoreError::Conflict);
            }
        }
        _ => return Err(StoreError::Invalid),
    }
    check_counts(transaction, reservation.job_id().as_str()).await?;
    sqlx::query(
        "INSERT INTO coding_budget_reservations(job_id, reservation_sequence) VALUES (?, ?)",
    )
    .bind(reservation.job_id().as_str())
    .bind(sequence(reservation.reservation_sequence())?)
    .execute(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

async fn check_counts(
    transaction: &mut Transaction<'_, Sqlite>,
    job_id: &str,
) -> Result<(), StoreError> {
    let total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM coding_budget_reservations WHERE job_id=?")
            .bind(job_id)
            .fetch_one(&mut **transaction)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    let unsettled: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM coding_budget_reservations WHERE job_id=? AND settled_by_sequence IS NULL")
        .bind(job_id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    if total >= MAX_RESERVATIONS || unsettled >= MAX_UNSETTLED_RESERVATIONS {
        return Err(StoreError::LimitExceeded);
    }
    Ok(())
}

async fn settle_reservation(
    transaction: &mut Transaction<'_, Sqlite>,
    settlement: &BudgetSettlementV1,
) -> Result<(), StoreError> {
    let row = sqlx::query("SELECT settled_by_sequence, event_json FROM coding_budget_reservations r JOIN coding_events e ON e.job_id=r.job_id AND e.sequence=r.reservation_sequence WHERE r.job_id=? AND r.reservation_sequence=?")
        .bind(settlement.job_id().as_str())
        .bind(sequence(settlement.reservation_sequence())?)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?
        .ok_or(StoreError::NotFound)?;
    if row
        .try_get::<Option<i64>, _>("settled_by_sequence")
        .map_err(|_| StoreError::Invalid)?
        .is_some()
    {
        return Err(StoreError::Conflict);
    }
    let json: String = row.try_get("event_json").map_err(|_| StoreError::Invalid)?;
    let reservation = super::budget_records::reservation_from_json(
        json.as_bytes(),
        settlement.job_id(),
        settlement.reservation_sequence(),
    )?;
    if !matching_keys(&reservation, settlement) {
        return Err(StoreError::Conflict);
    }
    let changed = sqlx::query("UPDATE coding_budget_reservations SET settled_by_sequence=? WHERE job_id=? AND reservation_sequence=? AND settled_by_sequence IS NULL")
        .bind(sequence(settlement.settlement_sequence())?)
        .bind(settlement.job_id().as_str())
        .bind(sequence(settlement.reservation_sequence())?)
        .execute(&mut **transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    if changed.rows_affected() != 1 {
        return Err(StoreError::Conflict);
    }
    Ok(())
}

fn sequence(value: NonZeroU64) -> Result<i64, StoreError> {
    i64::try_from(value.get()).map_err(|_| StoreError::LimitExceeded)
}
