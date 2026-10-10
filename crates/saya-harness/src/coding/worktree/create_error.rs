//! Typed create failures for the managed-checkout create flow; the displays
//! never embed paths or bytes.

use crate::coding::budget::SequencerError;
use crate::coding::git::GitError;
use crate::coding::snapshot::SnapshotError;
use saya_store::StoreError;
use thiserror::Error;

/// Typed create failures; displays never embed paths or bytes.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CreateCheckoutError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    #[error(transparent)]
    Sequencer(#[from] SequencerError),
    #[error("the plan manifest does not match the published snapshot")]
    PlanDigestMismatch,
    #[error("the checkout id is terminally settled; use a new checkout id")]
    Terminal,
    #[error("the input does not match the recorded checkout ownership")]
    InputMismatch,
    #[error("the job has reached its per-job checkout cap")]
    CheckoutCapReached,
    #[error("the checkout id is owned by another job")]
    CheckoutIdInUse,
    #[error("the job reached its checkout operation limit")]
    OperationCapReached,
    #[error("the job is not running")]
    JobNotRunning,
    #[error("the job is bound to a different repository or snapshot")]
    JobBindingMismatch,
    #[error("the agent is not registered for the job")]
    AgentNotRegistered,
}
