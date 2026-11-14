//! Unknown usage of a model request is a different fact from an unknown
//! effect. A model request cannot change the workspace, so a cut, exhausted or
//! cancelled one leaves only its request, slot, attempt and token observations
//! unknown: they stay visible in the budget snapshot and the durable record,
//! but they never leave a settlement `Uncertain`, a child unsettled, a slot
//! held or a follow-up blocked. Tool, command and child-slot observations keep
//! that meaning.
//!
//! Records written before the two were separated settled such a request
//! `Uncertain` with only these observations unknown. They are never rewritten;
//! every reader derives the same interpretation from the observation
//! dimensions.
use saya_types::{BudgetDimension, BudgetEffectState, BudgetObservation, BudgetObservationValue};

/// The dimensions a model request settles: the logical request and its
/// concurrency slot, its transport attempts and its tokens.
fn is_usage_dimension(dimension: BudgetDimension) -> bool {
    matches!(
        dimension,
        BudgetDimension::LogicalRequests
            | BudgetDimension::ModelRequestSlots
            | BudgetDimension::TransportAttempts
            | BudgetDimension::InputTokens
            | BudgetDimension::OutputTokens
    )
}

/// Whether an observation outside the usage dimensions is unknown: an
/// effect, or retained capacity, that still needs reconciliation.
pub fn has_unknown_effect(observations: &[BudgetObservation]) -> bool {
    observations.iter().any(|observation| {
        observation.value() == BudgetObservationValue::Unknown
            && !is_usage_dimension(observation.dimension())
    })
}

/// Whether an observation inside the usage dimensions is unknown.
pub(super) fn has_unknown_usage(observations: &[BudgetObservation]) -> bool {
    observations.iter().any(|observation| {
        observation.value() == BudgetObservationValue::Unknown
            && is_usage_dimension(observation.dimension())
    })
}

/// `Uncertain` exactly when an effect observation is unknown; unknown usage
/// alone settles.
pub(super) fn effect_state(observations: &[BudgetObservation]) -> BudgetEffectState {
    if has_unknown_effect(observations) {
        BudgetEffectState::Uncertain
    } else {
        BudgetEffectState::Settled
    }
}
