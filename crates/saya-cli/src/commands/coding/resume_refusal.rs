//! The refusal for a job this application owns whose recovery facts leave no
//! resume: "not owned" denied a job that is ours and had merely ended.

use super::{CodingApplication, CodingError};
use saya_harness::coding::coordinator::inspect_recovery;
use saya_store::CodingRecoveryStatus;
use saya_types::{JobId, JobState};

impl CodingApplication {
    /// The verdict a resume needs before it asks anything or writes anything:
    /// read-only, and a job that is not clear is refused here, with nothing
    /// appended. An abandoned wait — the one blocker whose owner is gone — is
    /// closed first, and the verdict is read again after that. A job a live run
    /// still owns is refused too: an open run between its turns holds the
    /// folder with no recovery fact open, and its next append needs the very
    /// sequence any write here would take.
    pub(super) async fn clear_for_resume(
        &self,
        job: &JobId,
    ) -> Result<CodingRecoveryStatus, CodingError> {
        let status = self.recovery(job).await?;
        if !status.clear_for_resume() {
            if !solely_pending_approvals(&status) {
                return Err(CodingError::not_resumable(job, &status));
            }
            self.close_stale_approvals(job).await?;
            let status = self.recovery(job).await?;
            if !status.clear_for_resume() {
                // A request still waiting for its answer is another resume's
                // live question, not a fact that leaves this job no resume:
                // that is busy.
                if solely_pending_approvals(&status) {
                    return Err(CodingError::Busy);
                }
                return Err(CodingError::not_resumable(job, &status));
            }
            return self.unowned(job, status);
        }
        self.unowned(job, status)
    }

    /// Hands the verdict back only when no live run holds the job; nothing may
    /// be asked of or written into a job whose owner is alive, whatever its
    /// recovery facts say.
    fn unowned(
        &self,
        job: &JobId,
        status: CodingRecoveryStatus,
    ) -> Result<CodingRecoveryStatus, CodingError> {
        if self.live_run_holds(job)? {
            return Err(CodingError::InPlaceRootBusy);
        }
        Ok(status)
    }

    /// Whether a run of `job` that has not ended holds it, read-only. A run
    /// numbers its own events from memory, so an append it did not write makes
    /// its next append a conflict; an open run between its turns holds the
    /// folder with no recovery fact open, which is the one live state neither
    /// the verdict nor the state row names.
    ///
    /// This process knows its own open run exactly. For another process only
    /// the folder lease is readable, and while it is held the folder may be a
    /// run's: a lease this process holds itself is a job object it retains —
    /// the open-run check above already answered for that — so it is not read
    /// as another run's.
    pub(super) fn live_run_holds(&self, job: &JobId) -> Result<bool, CodingError> {
        if super::live_runs::run_is_open(job.as_str()) {
            return Ok(true);
        }
        let Some(local) = self.root.local() else {
            return Ok(false);
        };
        let root_id = local.id().as_repository_id().as_str();
        Ok(super::in_place_lease::leased(&self.state_root, root_id)?
            && !super::live_runs::leased_here(root_id))
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
