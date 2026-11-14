use super::{
    accounting::{add_usage, observation, owner_error, settle_one, update_usage_coverage},
    admission::PermitKind,
    ledger::State,
};
use saya_agent::{EffectAdmissionError, EffectOutcome, EffectPermit, EffectSettlement};
use saya_types::{AgentId, BudgetDimension, InvocationId};

#[derive(Clone, Copy)]
pub(super) enum Settlement {
    Child(EffectOutcome),
}

pub(super) async fn finish(
    state: &mut State,
    permit: EffectPermit,
    settlement: Settlement,
) -> Result<(), EffectAdmissionError> {
    let Some(stored) = state.permits.get(&permit.get()).cloned() else {
        return Err(state.owner_failure());
    };
    match (stored.kind, settlement) {
        (PermitKind::Child, Settlement::Child(outcome)) => {
            settle_one(
                state,
                permit.get(),
                vec![observation(BudgetDimension::ChildSlots, outcome, 1)],
            )
            .await?;
        }
        _ => return Err(state.owner_failure()),
    }
    Ok(())
}

pub(super) async fn finish_effect(
    state: &mut State,
    actor: Option<(&AgentId, &InvocationId)>,
    permit: EffectPermit,
    settlement: EffectSettlement,
) -> Result<(), saya_agent::EffectOwnerFailure> {
    // Only a tool is listed. A model request, and each of its transport
    // attempts, cannot change the workspace, so an unknown one leaves only its
    // usage unknown (counted in the snapshot and recorded in its settlement)
    // and the child settled.
    let effect_unknown = matches!(
        &settlement,
        EffectSettlement::Tool {
            outcome: EffectOutcome::Unknown
        }
    );
    let actor_key = actor.map(|(agent, invocation)| (agent.clone(), invocation.clone()));
    let Some(stored) = state.permits.get(&permit.get()).cloned() else {
        return Err(owner_error(state));
    };
    if stored.agent.as_ref().zip(stored.invocation.as_ref()) != actor {
        return Err(owner_error(state));
    }
    let result = match (stored.kind, settlement) {
        (
            PermitKind::Model {
                active_attempt: false,
                ..
            },
            EffectSettlement::ModelRequest { outcome, .. },
        ) => {
            let observations = vec![
                observation(BudgetDimension::LogicalRequests, outcome, 1),
                observation(BudgetDimension::ModelRequestSlots, outcome, 1),
            ];
            settle_one(state, permit.get(), observations)
                .await
                .map_err(|_| owner_error(state))?;
            if outcome == EffectOutcome::Unknown {
                state.snapshot.unknown_logical_requests =
                    state.snapshot.unknown_logical_requests.saturating_add(1);
                update_usage_coverage(state, outcome, false, false);
            }
            // The slot is released whatever the outcome: Saya has given up on
            // the request (retries exhausted, or cancellation settled) and no
            // longer waits on it. The provider may still bill it server-side,
            // which is why its usage stays unknown, but a request that cannot
            // change the workspace must not hold concurrency capacity forever.
            let Some(active) = state.counts.model_slots.checked_sub(1) else {
                return Err(owner_error(state));
            };
            state.counts.model_slots = active;
            Ok(())
        }
        (
            PermitKind::Physical { parent, attempt },
            EffectSettlement::PhysicalAttempt {
                attempt: actual,
                outcome,
                reported_usage,
                usage_complete,
            },
        ) if attempt == actual => {
            let Some(parent_permit) = state.permits.get(&parent).cloned() else {
                return Err(owner_error(state));
            };
            let PermitKind::Model {
                next_ordinal,
                call_kind,
                active_attempt: true,
            } = parent_permit.kind
            else {
                return Err(owner_error(state));
            };
            if call_kind != Some(attempt.kind()) || next_ordinal != attempt.ordinal() {
                return Err(owner_error(state));
            }
            let mut observations =
                vec![observation(BudgetDimension::TransportAttempts, outcome, 1)];
            observations.extend(super::token::observations(
                state,
                outcome,
                reported_usage,
                usage_complete,
            ));
            settle_one(state, permit.get(), observations)
                .await
                .map_err(|_| owner_error(state))?;
            if outcome == EffectOutcome::Unknown {
                state.snapshot.unknown_transport_attempts =
                    state.snapshot.unknown_transport_attempts.saturating_add(1);
            }
            if let Some(usage) = reported_usage {
                add_usage(state, usage, outcome, usage_complete);
            } else {
                update_usage_coverage(state, outcome, false, false);
            }
            let Some(next_ordinal) = next_ordinal.checked_add(1) else {
                return Err(owner_error(state));
            };
            let Some(parent_permit) = state.permits.get_mut(&parent) else {
                return Err(owner_error(state));
            };
            parent_permit.kind = PermitKind::Model {
                next_ordinal,
                call_kind,
                active_attempt: false,
            };
            super::token::settle(state, outcome, reported_usage, usage_complete)?;
            Ok(())
        }
        (PermitKind::Tool { group }, EffectSettlement::Tool { outcome }) => {
            super::tools::finish_tool(state, permit.get(), group, outcome)
                .await
                .map_err(|_| owner_error(state))
        }
        _ => Err(owner_error(state)),
    };
    if result.is_ok()
        && effect_unknown
        && let Some(key) = actor_key
    {
        state.unknown_children.insert(key);
    }
    result
}
