use super::{
    capacity::{
        allocate, bump, bump_local, check_base, check_id_capacity, check_limit, effect_permit,
        local_count,
    },
    ledger::State,
};
use saya_agent::{EffectAdmissionError, EffectPermit, PhysicalAttempt};
use saya_types::{AgentId, BudgetDimension, InvocationId};
use std::num::NonZeroU64;

#[derive(Clone)]
pub(super) struct StoredPermit {
    pub(super) reservation: NonZeroU64,
    pub(super) kind: PermitKind,
    pub(super) agent: Option<AgentId>,
    pub(super) invocation: Option<InvocationId>,
}

#[derive(Clone, Copy)]
pub(super) enum PermitKind {
    Model {
        next_ordinal: u8,
        call_kind: Option<saya_agent::EffectCallKind>,
        active_attempt: bool,
    },
    Physical {
        parent: u64,
        attempt: PhysicalAttempt,
    },
    Tool {
        group: NonZeroU64,
    },
    Child,
    Command,
}

impl StoredPermit {
    pub(super) fn is_tool(&self) -> bool {
        matches!(self.kind, PermitKind::Tool { .. })
    }
}

impl State {
    /// Admitted physical sends not yet settled; coverage may not read Complete
    /// while any of these are live.
    pub(super) fn pending_physical_attempts(&self) -> usize {
        self.permits
            .values()
            .filter(|permit| matches!(permit.kind, PermitKind::Physical { .. }))
            .count()
    }
}

pub(super) async fn admit_one(
    state: &mut State,
    dimension: BudgetDimension,
    amount: u64,
    agent: Option<AgentId>,
    invocation: Option<InvocationId>,
) -> Result<EffectPermit, EffectAdmissionError> {
    let charges = match dimension {
        BudgetDimension::ChildSlots => vec![(dimension, amount)],
        BudgetDimension::CommandSlots => vec![(dimension, amount)],
        _ => return Err(EffectAdmissionError::Denied),
    };
    check_base(state, 1)?;
    check_id_capacity(state, 1)?;
    check_limit(state, dimension, amount)?;
    let sequence = state
        .reserve(charges, agent.clone(), invocation.clone())
        .await?;
    bump(state, dimension, amount)?;
    let id = allocate(state)?;
    state.permits.insert(
        id,
        StoredPermit {
            reservation: sequence,
            kind: match dimension {
                BudgetDimension::CommandSlots => PermitKind::Command,
                _ => PermitKind::Child,
            },
            agent,
            invocation,
        },
    );
    effect_permit(id)
}

pub(super) async fn admit_model(
    state: &mut State,
    agent: Option<AgentId>,
    invocation: Option<InvocationId>,
    local_limit: Option<u64>,
) -> Result<EffectPermit, EffectAdmissionError> {
    check_base(state, 1)?;
    check_id_capacity(state, 1)?;
    check_limit(state, BudgetDimension::LogicalRequests, 1)?;
    check_limit(state, BudgetDimension::ModelRequestSlots, 1)?;
    if let Some(limit) = local_limit
        && local_count(state, &agent, &invocation, BudgetDimension::LogicalRequests)
            .is_some_and(|count| count >= limit)
    {
        return Err(EffectAdmissionError::Denied);
    }
    let sequence = state
        .reserve(
            vec![
                (BudgetDimension::LogicalRequests, 1),
                (BudgetDimension::ModelRequestSlots, 1),
            ],
            agent.clone(),
            invocation.clone(),
        )
        .await?;
    bump(state, BudgetDimension::LogicalRequests, 1)?;
    bump(state, BudgetDimension::ModelRequestSlots, 1)?;
    bump_local(
        state,
        &agent,
        &invocation,
        BudgetDimension::LogicalRequests,
        1,
    );
    let id = allocate(state)?;
    state.permits.insert(
        id,
        StoredPermit {
            reservation: sequence,
            kind: PermitKind::Model {
                next_ordinal: 0,
                call_kind: None,
                active_attempt: false,
            },
            agent,
            invocation,
        },
    );
    effect_permit(id)
}

pub(super) async fn admit_physical(
    state: &mut State,
    attempt: PhysicalAttempt,
    agent: Option<AgentId>,
    invocation: Option<InvocationId>,
    request: Option<&saya_agent::PhysicalRequestMetadata>,
) -> Result<EffectPermit, EffectAdmissionError> {
    check_base(state, 1)?;
    check_id_capacity(state, 1)?;
    check_limit(state, BudgetDimension::TransportAttempts, 1)?;
    let parent_id = attempt.parent().get();
    let Some(parent) = state.permits.get(&parent_id).cloned() else {
        return Err(state.owner_failure());
    };
    if parent.agent != agent || parent.invocation != invocation {
        return Err(state.owner_failure());
    }
    let PermitKind::Model {
        next_ordinal,
        call_kind,
        active_attempt,
    } = parent.kind
    else {
        return Err(state.owner_failure());
    };
    if active_attempt
        || attempt.ordinal() != next_ordinal
        || next_ordinal >= 16
        || call_kind.is_some_and(|kind| kind != attempt.kind())
    {
        return Err(state.owner_failure());
    }
    let token_bounds = super::token::reserve_amount(state, request)?;
    let mut charges = vec![(BudgetDimension::TransportAttempts, 1)];
    if let Some((input, output)) = token_bounds {
        charges.extend([
            (BudgetDimension::InputTokens, input),
            (BudgetDimension::OutputTokens, output),
        ]);
    }
    let sequence = state
        .reserve(charges, agent.clone(), invocation.clone())
        .await?;
    if let Some((input, output)) = token_bounds {
        state.counts.tokens = state
            .counts
            .tokens
            .checked_add(input + output)
            .ok_or_else(|| state.owner_failure())?;
    }
    bump(state, BudgetDimension::TransportAttempts, 1)?;
    let id = allocate(state)?;
    let Some(parent) = state.permits.get_mut(&parent_id) else {
        return Err(state.owner_failure());
    };
    parent.kind = PermitKind::Model {
        next_ordinal,
        call_kind: Some(attempt.kind()),
        active_attempt: true,
    };
    state.permits.insert(
        id,
        StoredPermit {
            reservation: sequence,
            kind: PermitKind::Physical {
                parent: parent_id,
                attempt,
            },
            agent,
            invocation,
        },
    );
    effect_permit(id)
}
