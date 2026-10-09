//! Mutations of a run's own row: creation, status transition, and the usage
//! mirror. The step table's mutations live beside this in [`super::steps`].

use crate::contracts::now;
use crate::runs::records::{NewRun, RunRecord, RunUsage, encode_endpoint_counts, i64_of};
use crate::runs::status::{RunStatus, failure_code_str};
use crate::{SqliteStateStore, StoreError};
use saya_types::{RunFailureCode, RunId};

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
