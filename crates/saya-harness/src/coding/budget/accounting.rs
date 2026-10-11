use super::estimates::add_estimate;
use super::ledger::State;
use saya_agent::{EffectAdmissionError, EffectOutcome, EffectOwnerFailure};
use saya_types::{BudgetDimension, BudgetEffectState, BudgetObservation, BudgetObservationValue};

pub(super) async fn settle_one(
    state: &mut State,
    id: u64,
    observations: Vec<BudgetObservation>,
) -> Result<(), EffectAdmissionError> {
    let Some(permit) = state.permits.get(&id).cloned() else {
        return Err(state.owner_failure());
    };
    let uncertain = observations
        .iter()
        .any(|item| item.value() == BudgetObservationValue::Unknown);
    super::reservations::settle_reservation(
        state,
        permit.reservation,
        if uncertain {
            BudgetEffectState::Uncertain
        } else {
            BudgetEffectState::Settled
        },
        observations,
    )
    .await?;
    state.permits.remove(&id);
    Ok(())
}

pub(super) fn observation(
    dimension: BudgetDimension,
    outcome: EffectOutcome,
    units: u64,
) -> BudgetObservation {
    let value = match outcome {
        EffectOutcome::Unknown => BudgetObservationValue::Unknown,
        EffectOutcome::Succeeded => BudgetObservationValue::Measured(units),
        EffectOutcome::NotStarted => BudgetObservationValue::Measured(0),
        _ => BudgetObservationValue::Unknown,
    };
    BudgetObservation::new(dimension, None, None, value)
        .expect("a count dimension with a scalar value is valid")
}

pub(super) fn owner_error(state: &mut State) -> EffectOwnerFailure {
    state.snapshot.poisoned = true;
    state
        .cancellation
        .cancel_with_reason(saya_agent::CancellationReason::OwnerFailure);
    EffectOwnerFailure::new(std::io::Error::other(
        "effect settlement did not match its live permit",
    ))
}

pub(super) fn add_usage(
    state: &mut State,
    usage: saya_agent::TokenUsage,
    outcome: EffectOutcome,
    complete: bool,
) {
    if !complete {
        add_estimate(state, usage);
        update_usage_coverage(state, outcome, true, false);
        return;
    }
    let input = state
        .snapshot
        .reported_input_tokens
        .unwrap_or(0)
        .checked_add(usage.input_tokens);
    let output = state
        .snapshot
        .reported_output_tokens
        .unwrap_or(0)
        .checked_add(usage.output_tokens);
    let (Some(input), Some(output)) = (input, output) else {
        state.usage_ever_unknown = true;
        state.snapshot.usage_coverage = super::ledger::UsageCoverage::Unknown;
        return;
    };
    state.snapshot.reported_input_tokens = Some(input);
    state.snapshot.reported_output_tokens = Some(output);
    update_usage_coverage(state, outcome, true, complete);
}

/// Recomputes usage coverage from sticky observation facts rather than from the
/// previous coverage value. An unknown outcome keeps coverage Unknown forever; a
/// missing or incomplete reported total keeps it below Complete forever; and a
/// still-pending physical permit keeps it below Complete until it settles.
pub(super) fn update_usage_coverage(
    state: &mut State,
    outcome: EffectOutcome,
    reported: bool,
    complete: bool,
) {
    use super::ledger::UsageCoverage;
    if outcome == EffectOutcome::Unknown {
        state.usage_ever_unknown = true;
    }
    if !reported || !complete {
        state.usage_ever_incomplete = true;
    }
    state.snapshot.usage_coverage = if state.usage_ever_unknown {
        UsageCoverage::Unknown
    } else if state.usage_ever_incomplete || state.pending_physical_attempts() > 0 {
        UsageCoverage::Partial
    } else if reported {
        UsageCoverage::Complete
    } else {
        UsageCoverage::Unavailable
    };
}
