//! Stop-truth for the candidate orchestration: what a returned attempt's
//! output must carry when the question's token stopped around it, and how a
//! settled clarification is recognised.

use saya_agent::{AgentEvent, AgentOutput, CancellationReason};

/// Whether a returned attempt ended on a settled clarification: the loop's
/// clarification turn emits [`AgentEvent::ClarificationNeeded`] and then
/// completes, so a returned Ok output carrying the event is a finished pause
/// for input, not a mid-turn interruption.
pub(super) fn ended_on_settled_clarification(output: &AgentOutput) -> bool {
    output
        .events
        .iter()
        .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. }))
}

/// Carries the token's typed stop onto the retained output. Only a configured
/// budget stop changes the output: it is marked partial and its retained
/// events gain the one typed `AnsweringBudgetStopped` event naming the real
/// dimension — deduplicated, and never attached to an output that was already
/// truncated for some other reason (a legacy truncation keeps its own truth
/// and is not relabelled as a configured-budget stop). A User or OwnerFailure
/// stop, or no stop, leaves the output as is; the adapter classifies those
/// from the token itself.
pub(super) fn apply_stop_reason(output: &mut AgentOutput, reason: Option<CancellationReason>) {
    let Some(CancellationReason::Budget(dimension)) = reason else {
        return;
    };
    if output.truncated {
        return;
    }
    output.truncated = true;
    if !output
        .events
        .iter()
        .any(|event| matches!(event, AgentEvent::AnsweringBudgetStopped { .. }))
    {
        output
            .events
            .push(AgentEvent::answering_budget_stopped(dimension));
    }
}
