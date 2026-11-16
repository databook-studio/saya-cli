use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Structural state of a coding job. Pause and uncertainty require explicit
/// reconciliation before another invocation can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum JobState {
    Queued,
    Running,
    WaitingForApproval,
    Paused,
    Completed,
    Failed,
    Cancelled,
    Uncertain,
}

impl JobState {
    /// Applies only an ordinary lifecycle transition; reconciliation is owned
    /// by the durable runtime and cannot be represented by this operation.
    pub fn transition_to(self, next: Self) -> Result<Self, JobStateTransitionError> {
        use JobState::{
            Cancelled, Completed, Failed, Paused, Queued, Running, Uncertain, WaitingForApproval,
        };
        let allowed = matches!(
            (self, next),
            (Queued, Running | Failed | Cancelled)
                | (
                    Running,
                    WaitingForApproval | Paused | Completed | Failed | Cancelled | Uncertain
                )
                | (WaitingForApproval, Queued | Failed | Cancelled | Uncertain)
        );
        if allowed {
            Ok(next)
        } else {
            Err(JobStateTransitionError::InvalidTransition {
                from: self,
                to: next,
            })
        }
    }
}

/// A requested ordinary transition that violates the coding job lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum JobStateTransitionError {
    #[error("job cannot transition from {from:?} to {to:?}")]
    InvalidTransition { from: JobState, to: JobState },
}
