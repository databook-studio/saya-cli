//! The refusal for a job this application owns whose recovery facts leave no
//! resume: "not owned" denied a job that is ours and had merely ended.

use super::CodingError;
use saya_store::CodingRecoveryStatus;
use saya_types::{JobId, JobState};

impl CodingError {
    /// States how the job ended and what is left to do, instead of denying a
    /// job the application owns.
    pub(crate) fn not_resumable(job: &JobId, status: &CodingRecoveryStatus) -> Self {
        let outcome = match status.failure_class {
            Some(class) => format!("{}: {}", state_name(status.state), class.as_str()),
            None => state_name(status.state).to_owned(),
        };
        let open = status.active_children
            + status.unresolved_reservations
            + status.pending_approvals
            + status.unresolved_checkouts;
        let detail = if open == 0 {
            format!("finished ({outcome}); start a new job or follow up")
        } else {
            // An open fact is a reconciliation, not a resume: two writers, an
            // unsettled effect, or an unanswered approval still stands.
            let noun = if open == 1 { "fact" } else { "facts" };
            format!("cannot be resumed yet; it holds {open} unresolved recovery {noun} ({outcome})")
        };
        Self::NotResumable {
            job: job.as_str().to_owned(),
            detail,
        }
    }
}

/// The wire name of a job state, for messages that state how a job ended.
fn state_name(state: JobState) -> &'static str {
    match state {
        JobState::Queued => "queued",
        JobState::Running => "running",
        JobState::WaitingForApproval => "waiting_for_approval",
        JobState::Paused => "paused",
        JobState::Completed => "completed",
        JobState::Failed => "failed",
        JobState::Cancelled => "cancelled",
        JobState::Uncertain => "uncertain",
        _ => "unknown",
    }
}
