use super::ledger::{MAX_LIVE_PERMITS, State};
use saya_agent::{EffectAdmissionError, EffectPermit};
use saya_types::{AgentId, BudgetDimension, InvocationId};
use std::num::NonZeroU64;

pub(super) fn check_base(state: &mut State, slots: usize) -> Result<(), EffectAdmissionError> {
    if state.quiescent.load(std::sync::atomic::Ordering::Acquire)
        || (state.runtime_renewal_required && state.runtime_authority.is_some())
    {
        return Err(EffectAdmissionError::Denied);
    }
    if state.snapshot.poisoned {
        return Err(state.owner_failure());
    }
    if state.cancellation.is_cancelled() {
        return Err(EffectAdmissionError::Denied);
    }
    if state.deadline_expired() {
        state.latch_deadline_expiry();
        return Err(EffectAdmissionError::Denied);
    }
    if state
        .permits
        .len()
        .checked_add(slots)
        .is_none_or(|count| count > MAX_LIVE_PERMITS)
    {
        return Err(EffectAdmissionError::Denied);
    }
    Ok(())
}

pub(super) fn check_id_capacity(
    state: &mut State,
    count: usize,
) -> Result<(), EffectAdmissionError> {
    let count = u64::try_from(count).map_err(|_| EffectAdmissionError::Denied)?;
    if state.next_id.checked_add(count).is_none() {
        return Err(state.owner_failure());
    }
    Ok(())
}

pub(super) fn check_limit(
    state: &mut State,
    dimension: BudgetDimension,
    amount: u64,
) -> Result<(), EffectAdmissionError> {
    let (used, cap) = match dimension {
        BudgetDimension::LogicalRequests => (state.counts.logical, state.limits.logical_requests),
        BudgetDimension::TransportAttempts => {
            (state.counts.transport, state.limits.transport_attempts)
        }
        BudgetDimension::ToolCalls => (state.counts.tools, state.limits.tool_calls),
        BudgetDimension::ModelRequestSlots => (state.counts.model_slots, state.limits.model_slots),
        BudgetDimension::ChildSlots => (state.counts.child_slots, state.limits.child_slots),
        BudgetDimension::CommandSlots => {
            (state.counts.command_slots, super::ledger::MAX_COMMAND_SLOTS)
        }
        _ => return Err(EffectAdmissionError::Denied),
    };
    if used.checked_add(amount).is_none_or(|next| next > cap) {
        return Err(EffectAdmissionError::Denied);
    }
    Ok(())
}

pub(super) fn bump(
    state: &mut State,
    dimension: BudgetDimension,
    amount: u64,
) -> Result<(), EffectAdmissionError> {
    let value = match dimension {
        BudgetDimension::LogicalRequests => state.counts.logical,
        BudgetDimension::TransportAttempts => state.counts.transport,
        BudgetDimension::ToolCalls => state.counts.tools,
        BudgetDimension::ModelRequestSlots => state.counts.model_slots,
        BudgetDimension::ChildSlots => state.counts.child_slots,
        BudgetDimension::CommandSlots => state.counts.command_slots,
        _ => return Err(EffectAdmissionError::Denied),
    };
    let Some(next) = value.checked_add(amount) else {
        return Err(state.owner_failure());
    };
    match dimension {
        BudgetDimension::LogicalRequests => state.counts.logical = next,
        BudgetDimension::TransportAttempts => state.counts.transport = next,
        BudgetDimension::ToolCalls => state.counts.tools = next,
        BudgetDimension::ModelRequestSlots => state.counts.model_slots = next,
        BudgetDimension::ChildSlots => state.counts.child_slots = next,
        BudgetDimension::CommandSlots => state.counts.command_slots = next,
        _ => return Err(EffectAdmissionError::Denied),
    }
    Ok(())
}

pub(super) fn bump_local(
    state: &mut State,
    agent: &Option<AgentId>,
    invocation: &Option<InvocationId>,
    dimension: BudgetDimension,
    amount: u64,
) {
    if let (Some(agent), Some(invocation)) = (agent, invocation)
        && let Some(local) = state.local.get_mut(&(agent.clone(), invocation.clone()))
    {
        match dimension {
            BudgetDimension::LogicalRequests => {
                local.logical = local.logical.saturating_add(amount)
            }
            BudgetDimension::ToolCalls => local.tools = local.tools.saturating_add(amount),
            _ => {}
        }
    }
}

pub(super) fn local_count(
    state: &State,
    agent: &Option<AgentId>,
    invocation: &Option<InvocationId>,
    dimension: BudgetDimension,
) -> Option<u64> {
    let (Some(agent), Some(invocation)) = (agent, invocation) else {
        return None;
    };
    let local = state.local.get(&(agent.clone(), invocation.clone()))?;
    Some(match dimension {
        BudgetDimension::LogicalRequests => local.logical,
        BudgetDimension::ToolCalls => local.tools,
        _ => 0,
    })
}

pub(super) fn allocate(state: &mut State) -> Result<u64, EffectAdmissionError> {
    let id = state.next_id;
    state.next_id = state
        .next_id
        .checked_add(1)
        .ok_or_else(|| state.owner_failure())?;
    Ok(id)
}

pub(super) fn effect_permit(id: u64) -> Result<EffectPermit, EffectAdmissionError> {
    NonZeroU64::new(id)
        .map(EffectPermit::new)
        .ok_or(EffectAdmissionError::Denied)
}
