use std::num::NonZeroU64;

use saya_types::{CodingEventKind, CodingEventV1, JobId};
use sqlx::{Row, Sqlite, Transaction};

use super::events::{CodingEventAppend, append_event_in_tx};
use crate::{SqliteStateStore, StoreError};

impl SqliteStateStore {
    /// Records a durable, non-refunding reconciliation fact for a
    /// reservation whose effect is open or settled `Uncertain`, so
    /// recovery stops counting it as unresolved. The reservation's
    /// charges stay consumed: reconciliation never refunds or releases.
    pub async fn reconcile_coding_budget(
        &self,
        event: CodingEventV1,
    ) -> Result<CodingEventAppend, StoreError> {
        let CodingEventKind::BudgetReconciled {
            reservation_sequence,
            ..
        } = event.event()
        else {
            return Err(StoreError::Invalid);
        };
        let reservation_sequence = *reservation_sequence;
        let job_id = event.job_id().clone();
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
                reconcile_reservation(&mut transaction, &job_id, reservation_sequence, sequence)
                    .await
                    .map(|()| CodingEventAppend::Appended(sequence))
            }
            Err(error) => Err(error),
        };
        finish(transaction, outcome).await
    }
}

/// Marks one reservation reconciled after its reconciliation event was
/// appended. Refuses a missing or foreign reservation, an already
/// reconciled one, and one settled `Settled` (nothing to reconcile).
/// Allowed: an open reservation, or one settled `Uncertain`.
async fn reconcile_reservation(
    transaction: &mut Transaction<'_, Sqlite>,
    job_id: &JobId,
    reservation_sequence: NonZeroU64,
    reconciled_by: NonZeroU64,
) -> Result<(), StoreError> {
    let row = sqlx::query(
        "SELECT r.reconciled_by_sequence, r.settled_by_sequence, COALESCE(json_extract(s.event_json,'$.event.settlement.effect_state'),'') AS effect_state \
         FROM coding_budget_reservations r \
         LEFT JOIN coding_events s ON s.job_id=r.job_id AND s.sequence=r.settled_by_sequence \
         WHERE r.job_id=? AND r.reservation_sequence=?",
    )
    .bind(job_id.as_str())
    .bind(sequence(reservation_sequence)?)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?
    .ok_or(StoreError::Conflict)?;
    if row
        .try_get::<Option<i64>, _>("reconciled_by_sequence")
        .map_err(|_| StoreError::Invalid)?
        .is_some()
    {
        return Err(StoreError::Conflict);
    }
    // Reconcilable: no settlement at all (open) or a settlement that says
    // `uncertain`. A settled one has nothing to reconcile, and a settlement
    // reference that does not resolve to a recognised state is corruption
    // that reconciliation must not hide.
    let settled_by = row
        .try_get::<Option<i64>, _>("settled_by_sequence")
        .map_err(|_| StoreError::Invalid)?;
    let effect_state = row
        .try_get::<String, _>("effect_state")
        .map_err(|_| StoreError::Invalid)?;
    if settled_by.is_some() && effect_state != "uncertain" {
        return Err(StoreError::Conflict);
    }
    let changed = sqlx::query(
        "UPDATE coding_budget_reservations SET reconciled_by_sequence=? \
         WHERE job_id=? AND reservation_sequence=? AND reconciled_by_sequence IS NULL",
    )
    .bind(sequence(reconciled_by)?)
    .bind(job_id.as_str())
    .bind(sequence(reservation_sequence)?)
    .execute(&mut **transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if changed.rows_affected() != 1 {
        return Err(StoreError::Conflict);
    }
    Ok(())
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

fn sequence(value: NonZeroU64) -> Result<i64, StoreError> {
    i64::try_from(value.get()).map_err(|_| StoreError::LimitExceeded)
}
