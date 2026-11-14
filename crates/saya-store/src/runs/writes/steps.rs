//! Mutations of a run's step rows: the step lifecycle transition and the
//! step's usage mirror. The run row's own mutations live beside this in
//! [`super::run`].

use crate::contracts::now;
use crate::runs::records::{RunUsage, encode_endpoint_counts, i64_of};
use crate::runs::status::RunStepStatus;
use crate::{SqliteStateStore, StoreError};
use saya_types::RunId;

/// Insert or transition a step. A step's first sighting is pending or
/// running — never already finished; afterwards the step machine governs,
/// including `failed → running` for the engine's bounded retry.
pub(crate) async fn upsert_step(
    store: &SqliteStateStore,
    run: &RunId,
    step: usize,
    status: RunStepStatus,
) -> Result<(), StoreError> {
    let step = i64::try_from(step).map_err(|_| StoreError::LimitExceeded)?;
    let pool = store.pool().await.map_err(|_| StoreError::Unavailable)?;
    let mut tx = pool.begin().await.map_err(|_| StoreError::Unavailable)?;
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM runs WHERE id=?")
        .bind(run.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    if exists.is_none() {
        tx.rollback().await.map_err(|_| StoreError::Unavailable)?;
        return Err(StoreError::NotFound);
    }
    let current: Option<String> =
        sqlx::query_scalar("SELECT status FROM run_steps WHERE run_id=? AND step=?")
            .bind(run.as_str())
            .bind(step)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
    match current {
        None => {
            if !status.is_initial() {
                tx.rollback().await.map_err(|_| StoreError::Unavailable)?;
                return Err(StoreError::Invalid);
            }
            let stamp = now();
            if let Err(error) = sqlx::query("INSERT INTO run_steps(run_id, step, status, created_unix_ms, updated_unix_ms) VALUES (?, ?, ?, ?, ?)")
                .bind(run.as_str())
                .bind(step)
                .bind(status.as_str())
                .bind(stamp)
                .bind(stamp)
                .execute(&mut *tx)
                .await
            {
                tx.rollback().await.map_err(|_| StoreError::Unavailable)?;
                // A raced twin writer that inserted the same step between this
                // writer's read and insert surfaces as a unique violation: a
                // conflict with an existing record, not a missing store.
                if error
                    .as_database_error()
                    .is_some_and(|db| db.is_unique_violation())
                {
                    return Err(StoreError::Conflict);
                }
                return Err(StoreError::Unavailable);
            }
        }
        Some(current) => {
            let Some(current) = RunStepStatus::parse(&current) else {
                tx.rollback().await.map_err(|_| StoreError::Unavailable)?;
                return Err(StoreError::Invalid);
            };
            if !RunStepStatus::can_transition(current, status) {
                tx.rollback().await.map_err(|_| StoreError::Unavailable)?;
                return Err(StoreError::Conflict);
            }
            sqlx::query(
                "UPDATE run_steps SET status=?, updated_unix_ms=? WHERE run_id=? AND step=?",
            )
            .bind(status.as_str())
            .bind(now())
            .bind(run.as_str())
            .bind(step)
            .execute(&mut *tx)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        }
    }
    tx.commit().await.map_err(|_| StoreError::Unavailable)?;
    store.secure_files()?;
    Ok(())
}

/// Replace a step's accumulated usage, same mirror semantics as the run's.
pub(crate) async fn set_step_usage(
    store: &SqliteStateStore,
    run: &RunId,
    step: usize,
    usage: RunUsage,
) -> Result<(), StoreError> {
    let tokens = encode_endpoint_counts(&usage.tokens_per_endpoint)?;
    let step = i64::try_from(step).map_err(|_| StoreError::LimitExceeded)?;
    let pool = store.pool().await.map_err(|_| StoreError::Unavailable)?;
    let result = sqlx::query(
        "UPDATE run_steps SET usage_wall_clock_ms=?, usage_tokens_json=?, usage_turns=?, usage_tool_calls=?, updated_unix_ms=? WHERE run_id=? AND step=?",
    )
    .bind(i64_of(usage.wall_clock_ms)?)
    .bind(tokens)
    .bind(i64_of(usage.turns)?)
    .bind(i64_of(usage.tool_calls)?)
    .bind(now())
    .bind(run.as_str())
    .bind(step)
    .execute(pool)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    store.secure_files()?;
    Ok(())
}
