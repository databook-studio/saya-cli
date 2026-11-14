//! Owner-side preparation at the end of a direct-mode run: when the root loop
//! ends, the OWNER prepares the candidate once if the root working checkout W
//! changed, no child is live and no receipt exists for the current generation.
//! A prepared candidate is therefore always ready without the model being told
//! to prepare.
//!
//! A preparation failure is NOT a root failure: the run's own result is
//! returned unchanged and the job settles like any other unprepared candidate
//! (not ready, never failed). The refusal is still NAMED: the closed
//! `PreparationRefusal` below reaches the advisory view, so the operator and
//! the model see why the candidate is not ready — never the refusal's own text,
//! which can carry program names, paths and process output.

use super::{CodingError, NativeCodingJob};
use saya_agent::AgentOutput;

/// Why the owner's end-of-run preparation produced no ready candidate: a
/// CLOSED set of names, each with one fixed sentence. Only these two strings
/// ever leave the job, so no program text, path outside the workspace, stderr
/// or secret can ride along with the reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PreparationRefusal {
    /// W carries no change to prepare.
    NothingToPrepare,
    /// A child was still live, so W could still move under the candidate.
    ChildrenActive,
    /// The prepared candidate's own state was incomplete, malformed or stale.
    CandidateState,
    /// The run was cancelled before the owner could prepare.
    Cancelled,
    /// Another owner operation held the job's one-operation slot.
    Busy,
    /// The preparation did not complete for a reason with no narrower name.
    Internal,
}

impl PreparationRefusal {
    /// The closed name the view carries.
    pub(super) fn reason(self) -> &'static str {
        match self {
            Self::NothingToPrepare => "nothing_to_prepare",
            Self::ChildrenActive => "children_active",
            Self::CandidateState => "candidate_state",
            Self::Cancelled => "cancelled",
            Self::Busy => "busy",
            Self::Internal => "internal",
        }
    }

    /// The one short fixed sentence the view carries with the name.
    pub(super) fn detail(self) -> &'static str {
        match self {
            Self::NothingToPrepare => "the working checkout has no change to prepare",
            Self::ChildrenActive => "a child was still active when the owner prepared",
            Self::CandidateState => "the prepared candidate state was incomplete or stale",
            Self::Cancelled => "the run was cancelled before the owner prepared",
            Self::Busy => "another owner operation was in progress",
            Self::Internal => "the owner preparation did not complete",
        }
    }

    /// The closed refusal one failed preparation maps to. Only the variant is
    /// read: the error's own text is never copied, because it can name a
    /// program, a path or a process's output.
    fn of(error: &CodingError) -> Self {
        match error {
            CodingError::NothingToPrepare => Self::NothingToPrepare,
            CodingError::ChildrenActive => Self::ChildrenActive,
            CodingError::PreparedCandidateState | CodingError::StaleReview => Self::CandidateState,
            CodingError::Busy => Self::Busy,
            _ => Self::Internal,
        }
    }
}

impl NativeCodingJob {
    /// Records why the owner's preparation produced no ready candidate. The
    /// first refusal stands: the end-of-run hook prepares at most once, and a
    /// later advisory read must not overwrite what the run ended with.
    fn note_preparation(&self, refusal: Option<PreparationRefusal>) {
        let Some(refusal) = refusal else {
            return;
        };
        let mut held = self
            .preparation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        held.get_or_insert(refusal);
    }

    /// The closed refusal the run ended with, or `None` when the owner prepared
    /// a ready candidate, prepared nothing, or never had to prepare.
    pub(super) fn preparation_refusal(&self) -> Option<PreparationRefusal> {
        *self
            .preparation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The end-of-run hook: prepares the candidate through the existing
    /// `prepare_candidate` when — and only when — `preparation_needed` holds.
    /// The root run's result is passed through untouched; a refusal (a change
    /// outside the ceiling, a failed check, an unreadable record) leaves the
    /// final envelope reporting the advisory readiness, which says not ready
    /// and names the closed reason it is not ready.
    pub(super) async fn prepare_at_finish(
        &self,
        result: Result<AgentOutput, CodingError>,
    ) -> Result<AgentOutput, CodingError> {
        // An in-place root is the user's folder: remove the scratch tree at
        // job end, but only the known layout. Anything else is left alone.
        if self.application.is_in_place() {
            let _ = saya_harness::coding::worktree::remove_in_place_scratch(
                self.application.root_path(),
            );
        }
        let output = match result {
            Ok(output) => output,
            Err(error) => return Err(error),
        };
        // A headless `start` has nobody waiting on a ready candidate: `saya job
        // review` prepares one on demand from W, and readiness still names the
        // generation and reports it not ready. Skip the checkout-and-integrate
        // pass the operator never asked for.
        if self
            .deferred_preparation
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Ok(output);
        }
        if self.preparation_needed().await {
            // Nothing runs a check, so a completed preparation always
            // yields a ready candidate: only a failure leaves a refusal.
            if let Err(error) = self.prepare_candidate().await {
                let refusal = if self.cancellation.reason().is_some() {
                    PreparationRefusal::Cancelled
                } else {
                    PreparationRefusal::of(&error)
                };
                self.note_preparation(Some(refusal));
            }
        }
        Ok(output)
    }

    /// Whether the owner must prepare: no live child (a running child can still
    /// change W), a changed W, and no receipt for the current generation (a
    /// receipt is immutable per generation, so replaying one prepares nothing).
    async fn preparation_needed(&self) -> bool {
        if self.active.lock().await.is_some() {
            return false;
        }
        if self
            .root_changes()
            .await
            .map_or(true, |changes| changes.is_empty())
        {
            return false;
        }
        self.current_generation()
            .await
            .is_ok_and(|generation| !self.prepared_receipt_path(&generation.digest).exists())
    }
}
