//! The refusal for a job this application owns whose recovery facts leave no
//! resume: "not owned" denied a job that is ours and had merely ended.

use super::{CodingApplication, CodingError};
use saya_harness::coding::coordinator::inspect_recovery;
use saya_store::CodingRecoveryStatus;
use saya_types::{JobId, JobState};

impl CodingApplication {
    /// The recovery verdict a resume needs before it asks anything or writes
    /// anything: read-only, and a job that is not clear is refused here, with
    /// nothing appended. An abandoned wait — the one blocker whose owner is
    /// gone — is closed first, and the verdict is read again after that.
    pub(super) async fn clear_for_resume(
        &self,
        job: &JobId,
    ) -> Result<CodingRecoveryStatus, CodingError> {
        let status = self.recovery(job).await?;
        if status.clear_for_resume() {
            return Ok(status);
        }
        if !solely_pending_approvals(&status) {
            return Err(CodingError::not_resumable(job, &status));
        }
        self.close_stale_approvals(job).await?;
        let status = self.recovery(job).await?;
        if status.clear_for_resume() {
            return Ok(status);
        }
        // A request still waiting for its answer is another resume's live
        // question, not a fact that leaves this job no resume: that is busy.
        if solely_pending_approvals(&status) {
            return Err(CodingError::Busy);
        }
        Err(CodingError::not_resumable(job, &status))
    }

    async fn recovery(&self, job: &JobId) -> Result<CodingRecoveryStatus, CodingError> {
        inspect_recovery(&self.store, job)
            .await
            .map_err(CodingError::owner)
    }
}

/// Whether an approval request is the only open fact: a process that died
/// while waiting on a human, whose run is gone by construction, so closing it
/// writes into no live run. Every other open fact may belong to one.
fn solely_pending_approvals(status: &CodingRecoveryStatus) -> bool {
    status.pending_approvals != 0
        && status.active_children == 0
        && status.unresolved_reservations == 0
        && status.unresolved_checkouts == 0
}

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
