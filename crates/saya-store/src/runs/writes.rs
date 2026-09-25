//! Mutations for the run-spine tables.
//!
//! Every mutation is one honest operation: a transition that is not in the
//! state machine is `Conflict`, a record that does not exist is `NotFound`,
//! and a duplicate create is `Conflict` — the engine's single-writer lock
//! makes races rare, and the guarded `UPDATE ... WHERE status = ?` makes the
//! one that matters visible instead of silent.

use saya_types::{RunFailureCode, RunId};

use crate::contracts::now;
use crate::runs::records::{NewRun, RunRecord, RunUsage, encode_endpoint_counts, i64_of};
use crate::runs::status::{RunStatus, RunStepStatus, failure_code_str};
use crate::{SqliteStateStore, StoreError};

/// Create a run's metadata row. The run starts `Planned`; creating the same
/// id twice is `Conflict` — a resumed run is loaded, not re-created.
pub(crate) async fn create_run(
    store: &SqliteStateStore,
    run: NewRun,
) -> Result<RunRecord, StoreError> {
    let budget_tokens = encode_endpoint_counts(&run.budgets.tokens_per_endpoint)?;
    let stamp = now();
    let pool = store.pool().await.map_err(|_| StoreError::Unavailable)?;
    let result = sqlx::query(
        "INSERT INTO runs(id, status, failure_code, cap_workspace_write, cap_fetch, cap_runner, cap_scratch, budget_wall_clock_ms, budget_tokens_json, budget_turns, budget_tool_calls, budget_downloaded_bytes, budget_workspace_bytes, budget_workspace_files, budget_process_count, budget_process_time_ms, usage_wall_clock_ms, usage_tokens_json, usage_turns, usage_tool_calls, created_unix_ms, updated_unix_ms) VALUES (?, 'planned', NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, NULL, NULL, ?, ?) ON CONFLICT(id) DO NOTHING",
    )
    .bind(run.id.as_str())
    .bind(i64::from(run.capabilities.workspace_write))
    .bind(i64::from(run.capabilities.fetch))
    .bind(i64::from(run.capabilities.runner))
    .bind(i64::from(run.capabilities.scratch))
    .bind(i64_of(run.budgets.wall_clock_ms)?)
    .bind(budget_tokens)
    .bind(i64_of(run.budgets.turns)?)
    .bind(i64_of(run.budgets.tool_calls)?)
    .bind(i64_of(run.budgets.downloaded_bytes)?)
    .bind(i64_of(run.budgets.workspace_bytes)?)
    .bind(i64_of(run.budgets.workspace_files)?)
    .bind(i64_of(run.budgets.process_count)?)
    .bind(i64_of(run.budgets.process_time_ms)?)
    .bind(stamp)
    .bind(stamp)
    .execute(pool)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if result.rows_affected() == 0 {
        return Err(StoreError::Conflict);
    }
    let NewRun {
        id,
        capabilities,
        budgets,
    } = run;
    Ok(RunRecord {
        id,
        status: RunStatus::Planned,
        failure_code: None,
        capabilities,
        budgets,
        usage: RunUsage::unknown(),
        created_unix_ms: stamp,
        updated_unix_ms: stamp,
    })
}

/// Transition a run's status. The machine refuses first — an illegal
/// transition is `Conflict` however it is described — and then a legal
/// `Failed` must carry exactly its typed code while no other status carries
/// one (`Invalid`). The `UPDATE`'s `WHERE status = ?` guard keeps a raced
/// writer a visible `Conflict` instead of a silent overwrite.
pub(crate) async fn set_run_status(
    store: &SqliteStateStore,
    id: &RunId,
    status: RunStatus,
    failure_code: Option<RunFailureCode>,
) -> Result<(), StoreError> {
    let pool = store.pool().await.map_err(|_| StoreError::Unavailable)?;
    let current: Option<String> = sqlx::query_scalar("SELECT status FROM runs WHERE id=?")
        .bind(id.as_str())
        .fetch_optional(pool)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let Some(current) = RunStatus::parse(&current.ok_or(StoreError::NotFound)?) else {
        return Err(StoreError::Invalid);
    };
    if !RunStatus::can_transition(current, status) {
        return Err(StoreError::Conflict);
    }
    // A terminal failure carries exactly its typed cause; no other status
    // carries one.
    if (status == RunStatus::Failed) != failure_code.is_some() {
        return Err(StoreError::Invalid);
    }
    let code = match failure_code {
        Some(code) => Some(failure_code_str(code).ok_or(StoreError::Invalid)?),
        None => None,
    };
    let result = sqlx::query(
        "UPDATE runs SET status=?, failure_code=?, updated_unix_ms=? WHERE id=? AND status=?",
    )
    .bind(status.as_str())
    .bind(code)
    .bind(now())
    .bind(id.as_str())
    .bind(current.as_str())
    .execute(pool)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if result.rows_affected() == 0 {
        return Err(StoreError::Conflict);
    }
    store.secure_files()?;
    Ok(())
}

/// Replace the run's accumulated usage. Mirror semantics: the engine sends
/// the totals it holds, not a delta — an unreported figure is `None` and
/// stays NULL in the row.
pub(crate) async fn set_run_usage(
    store: &SqliteStateStore,
    id: &RunId,
    usage: RunUsage,
) -> Result<(), StoreError> {
    let tokens = encode_endpoint_counts(&usage.tokens_per_endpoint)?;
    let pool = store.pool().await.map_err(|_| StoreError::Unavailable)?;
    let result = sqlx::query(
        "UPDATE runs SET usage_wall_clock_ms=?, usage_tokens_json=?, usage_turns=?, usage_tool_calls=?, updated_unix_ms=? WHERE id=?",
    )
    .bind(i64_of(usage.wall_clock_ms)?)
    .bind(tokens)
    .bind(i64_of(usage.turns)?)
    .bind(i64_of(usage.tool_calls)?)
    .bind(now())
    .bind(id.as_str())
    .execute(pool)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    store.secure_files()?;
    Ok(())
}

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
