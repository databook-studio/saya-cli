//! The one-shot result object's vocabulary: the CC subtype a failure names,
//! the process code a terminal state states, and the durable state read back
//! after finalization.

use super::{CodingApplication, NativeCodingJob};
use saya_types::{CodingFailureClass, CodingRunFailure, JobState};
use serde_json::Value;

/// The result document and the process code it states.
#[derive(Debug, Clone)]
pub struct OneShotResult {
    pub value: Value,
    pub exit_code: i32,
}

/// The CC subtype one failure names. A clean run is `success`; a cost guard
/// is the budget stop, the logical-requests backstop is the turn stop, and
/// everything else is an execution error.
pub(super) fn subtype(failure: Option<&CodingRunFailure>) -> &'static str {
    match failure {
        None => "success",
        Some(failure) if failure.class == CodingFailureClass::Cost => "error_max_budget_usd",
        Some(failure)
            if failure.class == CodingFailureClass::Backstop
                && failure.limit.as_deref() == Some("limits.job.logical_requests") =>
        {
            "error_max_turns"
        }
        Some(_) => "error_during_execution",
    }
}

/// The process exit code the terminal state states.
pub(super) fn exit_code(state: JobState) -> i32 {
    match state {
        JobState::Completed => 0,
        JobState::Paused => 6,
        JobState::Cancelled => 130,
        // A state that cannot finish a run — queued, running, waiting, failed,
        // uncertain — is a failed one-shot.
        _ => 5,
    }
}

/// The authoritative terminal state the store recorded for the job. The
/// outcome is only a fallback for a record that could not be read; a job that
/// finished always has one.
pub(super) async fn state(app: &CodingApplication, job: &NativeCodingJob) -> JobState {
    if let Ok(Some(record)) = app.store().get_coding_job(job.job_id()).await {
        return record.state;
    }
    job.outcome()
        .and_then(|outcome| outcome.get("state").cloned())
        .and_then(|state| serde_json::from_value(state).ok())
        .unwrap_or(JobState::Failed)
}
