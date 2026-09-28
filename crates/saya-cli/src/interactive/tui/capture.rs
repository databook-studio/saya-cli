//! Ephemeral capture of the latest direct-`/sql` result and its execution
//! evidence.
//!
//! The capture lives on `App`, in memory, for this session only:
//! [`CapturedResult`] has no serde derives, and the session save payload the
//! TUI writes (`SessionState::redacted()`) never sees this type, so captured
//! rows cannot reach disk. Over the accounted budget the result is not
//! captured, the previous capture is cleared, and one system line names the
//! way out (`/export --refresh` re-runs the query). The walk stops the moment
//! the total passes the budget — a refusal never serializes the result to
//! size it.

use super::transcript::{BlockKind, Transcript};
use crate::agent::tools::AgentCapture;
use crate::config::runtime::RuntimeConfig;
use saya_types::{
    EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs, QueryResult, ResultScope,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Accounted-bytes ceiling for one captured result: above it the result
/// renders as usual but is not held — the way out is re-running it.
pub(crate) const CAPTURE_BUDGET_BYTES: usize = 32 * 1024 * 1024;

/// The latest direct-`/sql` result and its execution evidence. No serde
/// derives: this type must never enter a session file, a snapshot, or any
/// persisted record.
///
/// Written here on every successful `/sql`; read by `/export --snapshot`
/// (S11), which exports the result the user already inspected, and by the
/// report (S12) later.
pub(crate) struct CapturedResult {
    pub(crate) result: QueryResult,
    pub(crate) evidence: ExecutionEvidence,
}

/// Process-wide counter feeding [`ExecutionEvidence::new_execution_id`].
static EXECUTION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Current wall clock in unix milliseconds (0 before the epoch).
pub(crate) fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// The time-of-day of a unix-millisecond timestamp, `HH:MM:SS`, UTC —
/// deterministic and dependency-free. The snapshot export message labels it
/// UTC so it never reads as local wall time.
pub(crate) fn clock_hh_mm_ss(unix_ms: i64) -> String {
    let seconds = (unix_ms / 1000).rem_euclid(86_400);
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

/// The next execution id: process-wide counter + the task's dispatch time.
pub(crate) fn next_execution_id(started_unix_ms: i64) -> String {
    ExecutionEvidence::new_execution_id(
        started_unix_ms,
        EXECUTION_COUNTER.fetch_add(1, Ordering::Relaxed),
    )
}

/// Direct-SQL evidence for a successful result, or `None` when the profile's
/// dialect cannot be resolved — capture never guesses a dialect.
pub(crate) fn direct_sql_evidence(
    profile: Option<&str>,
    connection: Option<&str>,
    result: &QueryResult,
    runtime: &RuntimeConfig,
    started_unix_ms: i64,
) -> Option<ExecutionEvidence> {
    let dialect = runtime.named_profile(profile?).ok()?.dialect();
    let connection_label = connection
        .filter(|label| !label.is_empty())
        .map(str::to_owned)
        .or_else(|| profile.map(str::to_owned))
        .unwrap_or_else(|| "default".to_owned());
    Some(ExecutionEvidence::for_result(
        result,
        ExecutionEvidenceArgs {
            execution_id: next_execution_id(started_unix_ms),
            connection_label,
            connection_identity: None,
            dialect,
            max_rows: runtime.resolved.max_rows,
            started_unix_ms,
            finished_unix_ms: unix_now_ms(),
            source: EvidenceSource::DirectSql,
        },
    ))
}

/// Execution evidence for a promoted agent capture (D12): source `Agent`,
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

/// Accounted bytes of `result` against [`CAPTURE_BUDGET_BYTES`]; `None` when
/// over.
pub(crate) fn accounted_bytes(result: &QueryResult) -> Option<usize> {
    accounted_bytes_within(result, CAPTURE_BUDGET_BYTES)
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

/// Captures `result` when `accounted` — [`accounted_bytes`]'s verdict — is
/// `Some`; `None` clears any previous capture and pushes one visible system
/// line naming the way out. Production passes [`accounted_bytes`]; tests a
/// small-budget walk.
pub(crate) fn capture_within(
    captured: &mut Option<CapturedResult>,
    result: QueryResult,
    evidence: ExecutionEvidence,
    accounted: Option<usize>,
    transcript: &mut Transcript,
) {
    if accounted.is_some() {
        *captured = Some(CapturedResult { result, evidence });
        return;
    }
    *captured = None;
    transcript.push(
        BlockKind::System,
        format!(
            "Result not captured (larger than {}); /export --refresh will re-run it.",
            human_bytes(CAPTURE_BUDGET_BYTES)
        ),
    );
}

/// The budget's own size for the refusal line, derived from
/// [`CAPTURE_BUDGET_BYTES`] so message and limit cannot drift. `pub(crate)`:
/// the agent-capture side (`capture_agent`) derives its refusal wording from
/// the same budget.
pub(crate) fn human_bytes(bytes: usize) -> String {
    match (
        bytes.is_multiple_of(1024 * 1024),
        bytes.is_multiple_of(1024),
    ) {
        (true, _) => format!("{} MiB", bytes / (1024 * 1024)),
        (false, true) => format!("{} KiB", bytes / 1024),
        _ => format!("{bytes} B"),
    }
}
