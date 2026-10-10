use super::{
    admission::{PermitKind, StoredPermit},
    capacity,
    ledger::State,
    reservations::settle_reservation,
};
use saya_agent::{EffectAdmissionError, EffectOutcome, EffectPermit, ToolEffect};
use saya_types::{BudgetDimension, BudgetEffectState, BudgetObservation, BudgetObservationValue};
use std::{collections::HashSet, num::NonZeroU64};

#[derive(Clone)]
pub(super) struct ToolGroup {
    reservation: NonZeroU64,
    actor: Option<(saya_types::AgentId, saya_types::InvocationId)>,
    members: HashSet<u64>,
    total: u64,
    pending: usize,
    succeeded: u64,
    unknown: bool,
}

pub(super) async fn admit_tools(
    state: &mut State,
    effects: &[ToolEffect],
    agent: Option<saya_types::AgentId>,
    invocation: Option<saya_types::InvocationId>,
    local_limit: Option<u64>,
) -> Result<Vec<EffectPermit>, EffectAdmissionError> {
    if effects.is_empty() {
        return Ok(Vec::new());
    }
    if effects.len() > saya_agent::MAX_TOOL_CALLS_PER_RESPONSE {
        return Err(EffectAdmissionError::Denied);
    }
    let amount = u64::try_from(effects.len()).map_err(|_| EffectAdmissionError::Denied)?;
    capacity::check_base(state, effects.len())?;
    capacity::check_id_capacity(state, effects.len())?;
    capacity::check_limit(state, BudgetDimension::ToolCalls, amount)?;
    if let Some(limit) = local_limit
        && capacity::local_count(state, &agent, &invocation, BudgetDimension::ToolCalls)
            .is_some_and(|count| count.checked_add(amount).is_none_or(|next| next > limit))
    {
        return Err(EffectAdmissionError::Denied);
    }

    let sequence = state
        .reserve(
            vec![(BudgetDimension::ToolCalls, amount)],
            agent.clone(),
            invocation.clone(),
        )
        .await?;
    capacity::bump(state, BudgetDimension::ToolCalls, amount)?;
    capacity::bump_local(
        state,
        &agent,
        &invocation,
        BudgetDimension::ToolCalls,
        amount,
    );

    let mut ids = Vec::with_capacity(effects.len());
    let mut members = HashSet::with_capacity(effects.len());
    for _ in effects {
        let id = capacity::allocate(state)?;
        members.insert(id);
        ids.push(id);
    }
    state.groups.insert(
        sequence,
        ToolGroup {
            reservation: sequence,
            actor: agent.clone().zip(invocation.clone()),
            members: members.clone(),
            total: amount,
            pending: effects.len(),
            succeeded: 0,
            unknown: false,
        },
    );
    for id in &ids {
        state.permits.insert(
            *id,
            StoredPermit {
                reservation: sequence,
                kind: PermitKind::Tool { group: sequence },
                agent: agent.clone(),
                invocation: invocation.clone(),
            },
        );
    }
    ids.into_iter().map(capacity::effect_permit).collect()
}

pub(super) async fn finish_open_groups(
    state: &mut State,
    key: &(saya_types::AgentId, saya_types::InvocationId),
) -> Result<(), EffectAdmissionError> {
    let groups: Vec<_> = state
        .groups
        .iter()
        .filter_map(|(id, group)| {
            (group.actor.as_ref() == Some(key)).then_some((*id, group.clone()))
        })
        .collect();
    for (id, mut group) in groups {
        group.unknown = true;
        group.pending = 0;
        finalize_group(state, id, group).await?;
    }
    Ok(())
}

pub(super) async fn finish_tool(
    state: &mut State,
    permit_id: u64,
    group_id: NonZeroU64,
    outcome: EffectOutcome,
) -> Result<(), EffectAdmissionError> {
    let Some(mut group) = state.groups.get(&group_id).cloned() else {
        return Err(state.owner_failure());
    };
    if !group.members.remove(&permit_id) || group.pending == 0 {
        return Err(state.owner_failure());
    }
    group.pending -= 1;
    match outcome {
        EffectOutcome::Succeeded => group.succeeded = group.succeeded.saturating_add(1),
        EffectOutcome::Unknown => group.unknown = true,
        EffectOutcome::NotStarted => {}
        _ => group.unknown = true,
    }

    if group.pending == 0 {
        finalize_group(state, group_id, group).await?;
    } else {
        state.groups.insert(group_id, group);
    }
    state.permits.remove(&permit_id);
    Ok(())
}

async fn finalize_group(
    state: &mut State,
    id: NonZeroU64,
    group: ToolGroup,
) -> Result<(), EffectAdmissionError> {
    let measured = state
        .snapshot
        .measured_tool_calls
        .checked_add(group.succeeded)
        .ok_or_else(|| state.owner_failure())?;
    let unknown = if group.unknown {
        state
            .snapshot
            .unknown_tool_calls
            .checked_add(group.total)
            .ok_or_else(|| state.owner_failure())?
    } else {
        state.snapshot.unknown_tool_calls
    };
    let value = if group.unknown {
        BudgetObservationValue::Unknown
    } else {
        BudgetObservationValue::Measured(group.succeeded)
    };
    let observation = BudgetObservation::new(BudgetDimension::ToolCalls, None, None, value)
        .map_err(|_| state.owner_failure())?;
    settle_reservation(
        state,
        group.reservation,
        if group.unknown {
            BudgetEffectState::Uncertain
        } else {
            BudgetEffectState::Settled
        },
        vec![observation],
    )
    .await?;

    state.snapshot.measured_tool_calls = measured;
    state.snapshot.unknown_tool_calls = unknown;
    state.groups.remove(&id);
    for member in group.members {
        state.permits.remove(&member);
    }
    Ok(())
}
