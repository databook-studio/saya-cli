//! Shared identity and lifecycle contracts for native coding jobs.

mod check_receipts;
mod digest;
mod events;
mod ids;
mod jobs;
mod receipts;

pub use check_receipts::{
    CHECK_RECEIPT_VERSION, CheckReceiptV1, CodingCheckOutcome, MAX_CHECK_RECEIPT_BYTES,
};
pub use digest::{DigestError, Sha256Digest};
pub use events::{
    CODING_EVENT_VERSION, CodingEventError, CodingEventKind, CodingEventV1, MAX_CODING_EVENT_BYTES,
};
pub use ids::{AgentId, InvocationId, JobId, ManagedCheckoutId, RepositoryId, SnapshotId};
pub use jobs::{JobState, JobStateTransitionError};
pub use receipts::{
    CodingResultOutcome, MAX_RECEIPT_PATH_BYTES, MAX_RECEIPT_PATH_SET_BYTES, MAX_RECEIPT_PATHS,
    MAX_RECEIPT_REFERENCE_BYTES, MAX_RESULT_RECEIPT_BYTES, RESULT_RECEIPT_VERSION, ReceiptError,
    ResultReceiptV1,
};

use thiserror::Error;

/// Why a coding identifier failed its safe wire shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CodingIdError {
    #[error("{0} must be 1-128 ASCII letters, digits, hyphens, or underscores")]
    InvalidId(&'static str),
}
