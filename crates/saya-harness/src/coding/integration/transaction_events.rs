//! Durable journal plumbing for the apply-back flow: the admission
//! preflight, the sequencer closures and their probe-validated event
//! builders, and the outcome recorder.
//!
//! Event construction is relationship-only validation, so each builder is
//! proven constructible with a placeholder sequence before the sequencer
//! allocates. The closure rebuilds the event at the allocated sequence and,
//! on the impossible construction failure, returns the pre-validated probe
//! event instead; the sequencer then refuses it as `SequenceMismatch`
//! before any store call — the caller-bug class that leaves the ledger
//! owner usable.

use super::transaction::{ApplyBackError, ApplyBackInput};
use crate::coding::budget::SequencerError;
use saya_store::MAX_CHECKOUT_OPERATIONS_PER_JOB;
use saya_types::{
    CheckoutOperationState, CheckoutOperationV1, CodingEventKind, CodingEventV1, JobState,
    ManagedCheckoutLifecycle, RepositoryMutationAction,
};
use std::num::NonZeroU64;

/// Preflights the store's predictable intent refusals for an apply-back
/// run: the job must be running and under its per-job operation cap, and
/// the candidate checkout must be durably active or retained with no open
/// operation, owned by the agent this run claims. The store keeps the same
/// checks for races; refusing here means a refused append can never poison
/// the ledger owner.
pub(super) async fn preflight_apply_back_admission(
    input: &ApplyBackInput<'_>,
) -> Result<(), ApplyBackError> {
    match input.store.get_coding_job(input.job_id).await? {
        Some(job) if job.state == JobState::Running => {}
        _ => return Err(ApplyBackError::JobNotRunning),
    }
    if input.store.count_checkout_operations(input.job_id).await? >= MAX_CHECKOUT_OPERATIONS_PER_JOB
    {
        return Err(ApplyBackError::OperationCapReached);
    }
    let record = match input
        .store
        .get_managed_checkout(input.job_id, input.candidate.checkout_id())
        .await?
    {
        Some(record) => record,
        None => return Err(ApplyBackError::CandidateNotActive),
    };
    if !matches!(
        record.current_lifecycle,
        ManagedCheckoutLifecycle::Active | ManagedCheckoutLifecycle::Retained
    ) {
        return Err(ApplyBackError::CandidateNotActive);
    }
    if matches!(
        record.latest_operation.state(),
        CheckoutOperationState::Intent | CheckoutOperationState::Uncertain
    ) {
        return Err(ApplyBackError::CandidateBusy);
    }
    if input.agent_id != record.ownership.agent_id() {
        return Err(ApplyBackError::AgentMismatch);
    }
    Ok(())
}

/// Appends the durable `CheckoutOperationIntent` for the candidate's
/// apply-back run, returning the operation sequence the store accepted.
///
/// The event is validated with the placeholder sequence
/// [`NonZeroU64::MIN`] before the sequencer allocates; the closure rebuilds
/// the event at the allocated sequence and falls back to the probe on an
/// impossible construction failure. DTO construction can only fail on
/// inputs that can never validate; that surfaces as
/// [`SequencerError::SequenceMismatch`], the caller-bug class.
pub(super) async fn append_apply_back_intent(
    input: &ApplyBackInput<'_>,
) -> Result<NonZeroU64, ApplyBackError> {
    let probe = intent_event(input, NonZeroU64::MIN)?;
    let appended = input
        .sequencer
        .append_checkout_operation_intent(|sequence| {
            match NonZeroU64::new(sequence).and_then(|sequence| intent_event(input, sequence).ok())
            {
                Some(event) => event,
                None => probe.clone(),
            }
        })
        .await?;
    Ok(appended.sequence)
}

/// Records the apply-back outcome against the open operation. The apply-back
/// outcome policy records `Completed` only when every file is done; any
/// other end leaves the intent open and records nothing — recovery (W11b)
/// is the only other closer of the open `ApplyBack` intent. On the success
/// path the caller propagates errors.
pub(super) async fn record_apply_back_outcome(
    input: &ApplyBackInput<'_>,
    operation_sequence: NonZeroU64,
    state: CheckoutOperationState,
) -> Result<(), ApplyBackError> {
    // The probe validates at `operation + 1`; the placeholder value is never
    // zero, so the outer fallback below is unreachable. An operation
    // sequence at `u64::MAX` has no allocatable successor, so the probe —
    // and any real build — can only fail into the `SequenceMismatch`
    // caller-bug class.
    let probe_sequence =
        NonZeroU64::new(operation_sequence.get().checked_add(1).unwrap_or(u64::MAX))
            .unwrap_or(NonZeroU64::MIN);
    let probe = outcome_event(input, operation_sequence, state, probe_sequence)?;
    input
        .sequencer
        .append_checkout_outcome(|sequence| {
            match NonZeroU64::new(sequence)
                .and_then(|sequence| outcome_event(input, operation_sequence, state, sequence).ok())
            {
                Some(event) => event,
                None => probe.clone(),
            }
        })
        .await
        .map(|_| ())?;
    Ok(())
}

/// Builds the `CheckoutOperationIntent` event at `sequence`: the intent's
/// sequence is the operation's sequence, opened for the candidate checkout.
fn intent_event(
    input: &ApplyBackInput<'_>,
    sequence: NonZeroU64,
) -> Result<CodingEventV1, ApplyBackError> {
    let operation = CheckoutOperationV1::new(
        input.job_id.clone(),
        sequence,
        input.candidate.checkout_id().clone(),
        RepositoryMutationAction::ApplyBack,
        CheckoutOperationState::Intent,
    )
    .map_err(|_| unconstructible_event())?;
    CodingEventV1::new(
        sequence,
        input.job_id.clone(),
        input.agent_id.cloned(),
        None,
        CodingEventKind::CheckoutOperationIntent {
            operation: Box::new(operation),
        },
    )
    .map_err(|_| unconstructible_event())
}

/// Builds the `CheckoutOperationOutcome` event at `event_sequence` against
/// the open apply-back operation (`operation_sequence < event_sequence`).
fn outcome_event(
    input: &ApplyBackInput<'_>,
    operation_sequence: NonZeroU64,
    state: CheckoutOperationState,
    event_sequence: NonZeroU64,
) -> Result<CodingEventV1, ApplyBackError> {
    let operation = CheckoutOperationV1::new(
        input.job_id.clone(),
        operation_sequence,
        input.candidate.checkout_id().clone(),
        RepositoryMutationAction::ApplyBack,
        state,
    )
    .map_err(|_| unconstructible_event())?;
    CodingEventV1::new(
        event_sequence,
        input.job_id.clone(),
        input.agent_id.cloned(),
        None,
        CodingEventKind::CheckoutOperationOutcome {
            operation: Box::new(operation),
        },
    )
    .map_err(|_| unconstructible_event())
}

/// DTO construction can only fail on inputs that can never validate (every
/// field is a pre-validated type and the payload is far below the wire
/// limits); surface it as the sequencer's caller-bug class.
fn unconstructible_event() -> ApplyBackError {
    ApplyBackError::Sequencer(SequencerError::SequenceMismatch)
}
