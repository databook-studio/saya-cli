//! Execution evidence for the query results both surfaces show.
//!
//! A direct `/sql` and a promoted agent capture each carry provenance: what the
//! execution was bound to, how many rows the connector was allowed, and when it
//! ran. These functions build that evidence from the execution envelope alone,
//! never from a live runtime, so a configuration change between execution and
//! completion cannot rewrite what happened. The accounted walk bounds a result
//! before anything holds it: the walk stops the moment the total passes the
//! budget, so a refusal never serializes the result to size it.
//!
//! The old surface's in-memory capture of the last `/sql` result went with it in
//! X1; the new surface holds the result it shows in its own preview, so only the
//! evidence and the accounting are shared.

use crate::agent::tools::AgentCapture;
use crate::interactive::sql_execution::SqlExecution;
use saya_types::{
    EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs, QueryResult, ResultScope,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Accounted-bytes ceiling for one held result: above it a result renders as
/// usual but is not held — the way out is re-running it.
pub(crate) const CAPTURE_BUDGET_BYTES: usize = 32 * 1024 * 1024;

/// Process-wide counter feeding [`ExecutionEvidence::new_execution_id`].
static EXECUTION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Current wall clock in unix milliseconds (0 before the epoch).
pub(crate) fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// The next execution id: process-wide counter + the task's dispatch time.
pub(crate) fn next_execution_id(started_unix_ms: i64) -> String {
    ExecutionEvidence::new_execution_id(
        started_unix_ms,
        EXECUTION_COUNTER.fetch_add(1, Ordering::Relaxed),
    )
}

/// Direct-SQL evidence built ONLY from the executed envelope: the binding
/// (profile name, opaque identity, dialect, issued row cap) and the real
/// execution timestamps. Never reads the current runtime/App, so a context
/// change between execution and completion cannot rewrite the provenance.
pub(crate) fn direct_sql_evidence_from_execution(
    connection: Option<&str>,
    execution: &SqlExecution,
) -> ExecutionEvidence {
    let connection_label = connection
        .filter(|label| !label.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| execution.binding.profile_name.clone());
    ExecutionEvidence::for_result(
        &execution.result,
        ExecutionEvidenceArgs {
            execution_id: next_execution_id(execution.started_unix_ms),
            connection_label,
            connection_identity: Some(execution.binding.profile_identity.clone()),
            dialect: execution.binding.dialect,
            max_rows: execution.binding.applied_row_cap,
            started_unix_ms: execution.started_unix_ms,
            finished_unix_ms: execution.finished_unix_ms,
            source: EvidenceSource::DirectSql,
        },
    )
}

/// Execution evidence for a promoted agent capture: source `Agent`,
/// scope `ModelLimited` at the row cap the connector applied, labelled with
/// the connection that ran the query and its opaque profile identity, timed
/// by the capture itself.
pub(crate) fn agent_evidence(capture: &AgentCapture) -> ExecutionEvidence {
    let mut evidence = ExecutionEvidence::for_result(
        &capture.result,
        ExecutionEvidenceArgs {
            execution_id: next_execution_id(capture.started_unix_ms),
            connection_label: capture.connection.clone(),
            connection_identity: capture.profile_identity.clone(),
            dialect: capture.dialect,
            max_rows: capture.row_cap,
            started_unix_ms: capture.started_unix_ms,
            finished_unix_ms: capture.finished_unix_ms,
            source: EvidenceSource::Agent,
        },
    );
    evidence.scope = ResultScope::ModelLimited {
        row_cap: capture.row_cap,
    };
    evidence
}

/// Accounted bytes when within `budget`: column names plus every cell
/// (string byte length, 8 per number/bool/null, arrays and objects walked).
/// `None` the moment the total exceeds the budget — the walk stops there.
pub(crate) fn accounted_bytes_within(result: &QueryResult, budget: usize) -> Option<usize> {
    let mut total = 0usize;
    for column in &result.columns {
        total = add(total, column.len(), budget)?;
    }
    for row in &result.rows {
        total = add_value(total, row, budget)?;
    }
    Some(total)
}

/// Adds `bytes` to the total, refusing the walk once it would pass `budget`.
fn add(total: usize, bytes: usize, budget: usize) -> Option<usize> {
    let next = total.checked_add(bytes)?;
    (next <= budget).then_some(next)
}

fn add_value(total: usize, value: &serde_json::Value, budget: usize) -> Option<usize> {
    match value {
        serde_json::Value::String(text) => add(total, text.len(), budget),
        serde_json::Value::Number(_) | serde_json::Value::Bool(_) | serde_json::Value::Null => {
            add(total, 8, budget)
        }
        serde_json::Value::Array(items) => items
            .iter()
            .try_fold(total, |t, item| add_value(t, item, budget)),
        serde_json::Value::Object(map) => map.iter().try_fold(total, |t, (key, value)| {
            add_value(add(t, key.len(), budget)?, value, budget)
        }),
    }
}
