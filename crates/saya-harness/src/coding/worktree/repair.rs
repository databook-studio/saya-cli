//! Append only the outcome proved by reconciliation; never replay a Git effect.
use super::{CleanupError, CleanupInput};
use crate::coding::budget::SequencerError;
use saya_types::{
    CheckoutOperationState, CheckoutOperationV1, CodingEventKind, CodingEventV1,
    RepositoryMutationAction,
};
use std::num::NonZeroU64;

fn event(
    input: &CleanupInput<'_>,
    sequence: NonZeroU64,
    operation: NonZeroU64,
    action: RepositoryMutationAction,
    state: CheckoutOperationState,
) -> Result<CodingEventV1, CleanupError> {
    let operation = Box::new(
        CheckoutOperationV1::new(
            input.ownership.job_id.clone(),
            operation,
            input.ownership.checkout_id.clone(),
            action,
            state,
        )
        .map_err(|_| SequencerError::SequenceMismatch)?,
    );
    let kind = if state == CheckoutOperationState::Intent {
        CodingEventKind::CheckoutOperationIntent { operation }
    } else {
        CodingEventKind::CheckoutOperationOutcome { operation }
    };
    CodingEventV1::new(sequence, input.ownership.job_id.clone(), None, None, kind)
        .map_err(|_| SequencerError::SequenceMismatch.into())
}
pub(super) async fn append_intent(
    input: &CleanupInput<'_>,
    action: RepositoryMutationAction,
) -> Result<NonZeroU64, CleanupError> {
    let probe = event(
        input,
        NonZeroU64::MIN,
        NonZeroU64::MIN,
        action,
        CheckoutOperationState::Intent,
    )?;
    let result = input
        .sequencer
        .append_checkout_operation_intent(|number| {
            NonZeroU64::new(number)
                .and_then(|seq| event(input, seq, seq, action, CheckoutOperationState::Intent).ok())
                .unwrap_or(probe)
        })
        .await?;
    Ok(result.sequence)
}
pub(super) async fn append_outcome(
    input: &CleanupInput<'_>,
    operation: NonZeroU64,
    action: RepositoryMutationAction,
    state: CheckoutOperationState,
) -> Result<(), CleanupError> {
    let next = operation
        .get()
        .checked_add(1)
        .and_then(NonZeroU64::new)
        .ok_or(SequencerError::SequenceMismatch)?;
    let probe = event(input, next, operation, action, state)?;
    input
        .sequencer
        .append_checkout_outcome(|number| {
            NonZeroU64::new(number)
                .and_then(|seq| event(input, seq, operation, action, state).ok())
                .unwrap_or(probe)
        })
        .await?;
    Ok(())
}
