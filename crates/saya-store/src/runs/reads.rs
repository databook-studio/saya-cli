//! Reads for the run-spine tables.
//!
//! One query per read; profile-free because runs are not profile-scoped —
//! a run is identified by its id alone. A stored row this build cannot
//! decode (an unknown status, a negative figure, a malformed map) fails
//! closed as `Invalid` rather than guessing.

use saya_types::RunId;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use crate::runs::records::{
    RunBudgets, RunCapabilityFlags, RunRecord, RunStepRecord, RunSummary, RunUsage,
    decode_endpoint_counts, u64_of,
};
use crate::runs::status::{RunStatus, RunStepStatus, parse_failure_code};
use crate::{SqliteStateStore, StoreError};

/// The `runs` row, in select order. Twenty-two columns — above `query_as`'s
/// tuple support, so the row is decoded by column name.
const RUN_COLUMNS: &str = "id, status, failure_code, cap_workspace_write, cap_fetch, cap_runner, cap_scratch, budget_wall_clock_ms, budget_tokens_json, budget_turns, budget_tool_calls, budget_downloaded_bytes, budget_workspace_bytes, budget_workspace_files, budget_process_count, budget_process_time_ms, usage_wall_clock_ms, usage_tokens_json, usage_turns, usage_tool_calls, created_unix_ms, updated_unix_ms";

/// One column of the row, by name. The bounds are `try_get`'s own.
fn name<'r, T>(row: &'r SqliteRow, column: &str) -> Result<T, StoreError>
where
    T: sqlx::Decode<'r, sqlx::Sqlite> + sqlx::Type<sqlx::Sqlite>,
{
    row.try_get(column).map_err(|_| StoreError::Invalid)
}

fn decode_run(row: &SqliteRow) -> Result<RunRecord, StoreError> {
    let id: String = name(row, "id")?;
    let status: String = name(row, "status")?;
    let failure_code: Option<String> = name(row, "failure_code")?;
    Ok(RunRecord {
        id: RunId::parse(&id).map_err(|_| StoreError::Invalid)?,
        status: RunStatus::parse(&status).ok_or(StoreError::Invalid)?,
        failure_code: failure_code
            .map(|code| parse_failure_code(&code).ok_or(StoreError::Invalid))
            .transpose()?,
        capabilities: RunCapabilityFlags {
            workspace_write: name::<i64>(row, "cap_workspace_write")? != 0,
            fetch: name::<i64>(row, "cap_fetch")? != 0,
            runner: name::<i64>(row, "cap_runner")? != 0,
            scratch: name::<i64>(row, "cap_scratch")? != 0,
        },
        budgets: RunBudgets {
            wall_clock_ms: u64_of(name(row, "budget_wall_clock_ms")?)?,
            tokens_per_endpoint: decode_endpoint_counts(name(row, "budget_tokens_json")?)?,
            turns: u64_of(name(row, "budget_turns")?)?,
            tool_calls: u64_of(name(row, "budget_tool_calls")?)?,
            downloaded_bytes: u64_of(name(row, "budget_downloaded_bytes")?)?,
            workspace_bytes: u64_of(name(row, "budget_workspace_bytes")?)?,
            workspace_files: u64_of(name(row, "budget_workspace_files")?)?,
            process_count: u64_of(name(row, "budget_process_count")?)?,
            process_time_ms: u64_of(name(row, "budget_process_time_ms")?)?,
        },
        usage: RunUsage {
            wall_clock_ms: u64_of(name(row, "usage_wall_clock_ms")?)?,
            tokens_per_endpoint: decode_endpoint_counts(name(row, "usage_tokens_json")?)?,
            turns: u64_of(name(row, "usage_turns")?)?,
            tool_calls: u64_of(name(row, "usage_tool_calls")?)?,
        },
        created_unix_ms: name(row, "created_unix_ms")?,
        updated_unix_ms: name(row, "updated_unix_ms")?,
    })
}

/// One run's full metadata row.
pub(crate) async fn get_run(
    store: &SqliteStateStore,
    id: &RunId,
) -> Result<Option<RunRecord>, StoreError> {
    let sql = format!("SELECT {RUN_COLUMNS} FROM runs WHERE id=?");
    let row = sqlx::query(&sql)
        .bind(id.as_str())
        .fetch_optional(store.pool().await.map_err(|_| StoreError::Unavailable)?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    row.map(|row| decode_run(&row)).transpose()
}

/// Every run's summary, most recent first — the `saya run list` surface.
pub(crate) async fn list_runs(store: &SqliteStateStore) -> Result<Vec<RunSummary>, StoreError> {
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT id, status, created_unix_ms, updated_unix_ms FROM runs ORDER BY created_unix_ms DESC, id DESC",
    )
    .fetch_all(store.pool().await.map_err(|_| StoreError::Unavailable)?)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    rows.into_iter()
        .map(|(id, status, created, updated)| {
            Ok(RunSummary {
                id: RunId::parse(&id).map_err(|_| StoreError::Invalid)?,
                status: RunStatus::parse(&status).ok_or(StoreError::Invalid)?,
                created_unix_ms: created,
                updated_unix_ms: updated,
            })
        })
        .collect()
}

/// One `run_steps` row's columns, in select order.
type StepRow = (
    String,
    i64,
    String,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    i64,
    i64,
);

/// Every step of one run, in plan order — the resume scan reads this.
pub(crate) async fn list_steps(
    store: &SqliteStateStore,
    run: &RunId,
) -> Result<Vec<RunStepRecord>, StoreError> {
    let rows: Vec<StepRow> = sqlx::query_as(
        "SELECT run_id, step, status, usage_tokens_json, usage_wall_clock_ms, usage_turns, usage_tool_calls, created_unix_ms, updated_unix_ms FROM run_steps WHERE run_id=? ORDER BY step ASC",
    )
    .bind(run.as_str())
    .fetch_all(store.pool().await.map_err(|_| StoreError::Unavailable)?)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    rows.into_iter()
        .map(
            |(run_id, step, status, tokens, wall_clock, turns, tool_calls, created, updated)| {
                Ok(RunStepRecord {
                    run_id: RunId::parse(&run_id).map_err(|_| StoreError::Invalid)?,
                    step: usize::try_from(step).map_err(|_| StoreError::Invalid)?,
                    status: RunStepStatus::parse(&status).ok_or(StoreError::Invalid)?,
                    usage: RunUsage {
                        wall_clock_ms: u64_of(wall_clock)?,
                        tokens_per_endpoint: decode_endpoint_counts(tokens)?,
                        turns: u64_of(turns)?,
                        tool_calls: u64_of(tool_calls)?,
                    },
                    created_unix_ms: created,
                    updated_unix_ms: updated,
                })
            },
        )
        .collect()
}
