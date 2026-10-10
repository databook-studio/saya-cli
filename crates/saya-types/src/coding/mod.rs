//! Shared identity and lifecycle contracts for native coding jobs.

mod ids;
mod jobs;

pub use ids::{AgentId, InvocationId, JobId, ManagedCheckoutId, RepositoryId, SnapshotId};
pub use jobs::{JobState, JobStateTransitionError};

use thiserror::Error;

/// Why a coding identifier failed its safe wire shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CodingIdError {
    #[error("{0} must be 1-128 ASCII letters, digits, hyphens, or underscores")]
    InvalidId(&'static str),
}
