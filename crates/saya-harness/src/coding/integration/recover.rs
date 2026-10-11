//! Reconciliation of an open apply-back operation against its durable source binding.
//! Recovery never infers authority from a path or checksum in the local journal.

use std::{
    fmt,
    num::NonZeroU64,
    time::{Duration, Instant},
};

use saya_agent::CancellationToken;
use saya_types::{CheckoutOperationState, RepositoryMutationAction};

use super::{
    apply::detect_overlaps,
    recover_files::reconcile_file,
    recover_journal::RecoveryJournal,
    transaction::{ApplyBackError, ApplyBackInput},
    transaction_events::record_apply_back_outcome,
    transaction_identity::verify_apply_back_checkouts,
    transaction_journal::JournalError,
};
use crate::workspace::Workspace;

/// Finish the accepted change set, or restore only states left by that apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryMode {
    Finish,
    Restore,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RecoverError {
    #[error("there is no matching open apply-back operation")]
    NoOpenApplyBack,
    #[error("the apply-back inputs or a guarded effect were refused")]
    ApplyBack(#[from] ApplyBackError),
    #[error("the apply-back journal is unusable")]
    Journal(#[from] JournalError),
    #[error("recovery was cancelled")]
    Cancelled,
    #[error("recovery exceeded its deadline")]
    DeadlineExceeded,
    #[error("an interrupted restore must be resumed in restore mode")]
    RecoveryModeMismatch,
}

/// Counts only; artifacts and conflicted files remain available to the caller.
#[derive(Default)]
pub struct RecoveryOutcome {
    pub(super) finished: usize,
    pub(super) restored: usize,
    pub(super) conflicts: usize,
    pub(super) already_applied: usize,
}

impl RecoveryOutcome {
    pub fn finished(&self) -> usize {
        self.finished
    }
    pub fn restored(&self) -> usize {
        self.restored
    }
    pub fn conflicts(&self) -> usize {
        self.conflicts
    }
    pub fn already_applied(&self) -> usize {
        self.already_applied
    }
}

impl fmt::Debug for RecoveryOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecoveryOutcome")
            .field("finished", &self.finished)
            .field("restored", &self.restored)
            .field("conflicts", &self.conflicts)
            .field("already_applied", &self.already_applied)
            .finish()
    }
}

pub(super) fn check_gate(token: &CancellationToken, deadline: Instant) -> Result<(), RecoverError> {
    if token.is_cancelled() {
        return Err(RecoverError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(RecoverError::DeadlineExceeded);
    }
    Ok(())
}

/// Reconciles the entire accepted set, including an unjournaled suffix. A failed
/// effect leaves the intent open; a reconciled conflict closes it Failed and
/// retains artifacts. A repeated call after terminal outcome performs no writes.
pub async fn recover_apply_back(
    input: &ApplyBackInput<'_>,
    cancellation: &CancellationToken,
    timeout: Duration,
    mode: RecoveryMode,
) -> Result<RecoveryOutcome, RecoverError> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(RecoverError::DeadlineExceeded)?;
    check_gate(cancellation, deadline)?;
    let original = Workspace::open(input.repository.root())
        .map_err(|_| ApplyBackError::Apply(super::ApplyError::Fs))?;
    let _lease = super::transaction_lease::acquire(input.state_dir, &original)?;
    let record = input
        .store
        .get_managed_checkout(input.job_id, input.candidate.checkout_id())
        .await
        .map_err(ApplyBackError::from)?
        .ok_or(RecoverError::NoOpenApplyBack)?;
    if record.latest_operation.action() != RepositoryMutationAction::ApplyBack
        || !matches!(
            record.latest_operation.state(),
            CheckoutOperationState::Intent | CheckoutOperationState::Uncertain
        )
        || record.ownership.agent_id() != input.agent_id
    {
        return Err(RecoverError::NoOpenApplyBack);
    }
    let sequence: NonZeroU64 = record.latest_operation.operation_sequence();
    detect_overlaps(input.accepted).map_err(ApplyBackError::Overlap)?;
    verify_apply_back_checkouts(input, cancellation).await?;
    let mut journal = RecoveryJournal::open(input, sequence, &original)?;
    if mode == RecoveryMode::Finish && journal.has_restores() {
        return Err(RecoverError::RecoveryModeMismatch);
    }
    let mut outcome = RecoveryOutcome::default();
    for accepted in input.accepted {
        let child = Workspace::open(std::path::Path::new(accepted.verified().path().as_str()))
            .map_err(|_| ApplyBackError::Apply(super::ApplyError::Fs))?;
        for entry in accepted.delta().entries() {
            check_gate(cancellation, deadline)?;
            reconcile_file(
                input,
                &original,
                &child,
                entry,
                mode,
                &mut journal,
                &mut outcome,
            )?;
        }
    }
    check_gate(cancellation, deadline)?;
    let state = if mode == RecoveryMode::Finish && outcome.conflicts == 0 {
        CheckoutOperationState::Completed
    } else {
        CheckoutOperationState::Failed
    };
    record_apply_back_outcome(input, sequence, state).await?;
    Ok(outcome)
}
