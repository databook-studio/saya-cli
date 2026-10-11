//! Bounded one-shot dispatch. Durable results/restart and native caller are later owners.
use super::super::budget::JobBudgetLedger;
use super::{
    admit::{self, NodeState},
    events::ChildEvent,
    schedule_types::{
        MAX_SCHEDULE_EVENT_CAPACITY, ScheduleChildError, ScheduleError, ScheduleLimits,
        ScheduleOutcome, ScheduleReport, ScheduleStop, ScheduledChild,
    },
    scheduled_child,
};
use saya_agent::{CancellationReason, CancellationToken};
use std::collections::HashMap;
use tokio::{sync::mpsc, task::JoinSet};

/// Runs one trusted batch and joins its started work before normal return.
/// U01 must route all native job children here and bind provider keys to factories;
/// separate direct child calls bypass these in-memory path/provider leases.
/// Capacity leaves coordinator headroom, not a physically reserved model permit.
/// Root progress must be consumed promptly: full/closed queues cancel the batch.
/// Dropping this future aborts local tasks and fails the ledger closed; it does
/// not claim durable settlement, remote termination, or crash reconciliation.
pub async fn run_schedule(
    nodes: Vec<ScheduledChild>,
    limits: ScheduleLimits,
    ledger: &JobBudgetLedger,
    caller: CancellationToken,
    progress: mpsc::Sender<ChildEvent>,
) -> Result<ScheduleReport, ScheduleError> {
    let plans = admit::validate(&nodes, &ledger.job_id().await)?;
    let capacity = ledger.limits().await;
    let children =
        u64::try_from(limits.max_active_children).map_err(|_| ScheduleError::InvalidLimits)?;
    if children == 0
        || children > capacity.child_slots
        || children > capacity.model_slots.saturating_sub(1)
        || limits.max_active_per_provider == 0
        || limits.max_active_per_provider > limits.max_active_children
        || progress.max_capacity() > MAX_SCHEDULE_EVENT_CAPACITY
    {
        return Err(ScheduleError::InvalidLimits);
    }
    let deadline = tokio::time::Instant::now()
        .checked_add(ledger.remaining().await)
        .ok_or(ScheduleError::InvalidLimits)?;
    let owner = ledger.cancellation_token();
    if owner.is_cancelled() {
        return Err(ScheduleError::OwnerUnavailable);
    }
    let mut lease = ledger.try_schedule().ok_or(ScheduleError::Busy)?;
    if owner.is_cancelled() {
        return Err(ScheduleError::OwnerUnavailable);
    }
    if ledger.snapshot().await.child_slots_active != 0 {
        return Err(ScheduleError::ActiveChildren);
    }
    let cancellation = CancellationToken::new();
    let mut nodes = nodes.into_iter().map(Some).collect::<Vec<_>>();
    let mut states = vec![NodeState::Pending; nodes.len()];
    let mut children = JoinSet::new();
    let mut active = HashMap::new();
    let mut outcomes = Vec::with_capacity(nodes.len());
    let mut stop = None;
    let mut settled = true;
    loop {
        // Check cancellation before synchronous spawning, not just at the next wait.
        if stop.is_none() {
            let reason = if owner.is_cancelled() {
                owner.reason()
            } else if caller.is_cancelled() {
                caller.reason()
            } else if tokio::time::Instant::now() >= deadline {
                Some(CancellationReason::Budget(
                    saya_agent::BudgetDimension::ElapsedTime,
                ))
            } else {
                cancellation.reason()
            };
            if let Some(reason) = reason {
                stop = Some(
                    if owner.reason() == Some(CancellationReason::OwnerFailure) {
                        ScheduleStop::OwnerFailure
                    } else if reason == CancellationReason::OwnerFailure {
                        ScheduleStop::ChildFailed
                    } else {
                        ScheduleStop::Cancelled(reason)
                    },
                );
                cancellation.cancel_with_reason(reason);
            }
        }
        if stop.is_none() {
            for index in 0..nodes.len() {
                if admit::can_start(index, &plans, &states, limits) {
                    let Some(node) = nodes[index].take() else {
                        continue;
                    };
                    lease.mark_started();
                    states[index] = NodeState::Running;
                    let handle = children.spawn(scheduled_child::run(
                        node,
                        ledger.clone(),
                        cancellation.clone(),
                        progress.clone(),
                    ));
                    active.insert(handle.id(), index);
                }
            }
        }
        if children.is_empty() {
            if stop.is_none() && states.contains(&NodeState::Pending) {
                stop = Some(ScheduleStop::DependencyBlocked);
            }
            break;
        }
        let joined = tokio::select! {
            biased;
            _ = owner.cancelled(), if stop.is_none() => {
                cancellation.cancel_with_reason(owner.reason().unwrap_or(CancellationReason::OwnerFailure)); None
            }
            _ = caller.cancelled(), if stop.is_none() => {
                cancellation.cancel_with_reason(caller.reason().unwrap_or(CancellationReason::User)); None
            }
            _ = tokio::time::sleep_until(deadline), if stop.is_none() => {
                cancellation.cancel_with_reason(CancellationReason::Budget(saya_agent::BudgetDimension::ElapsedTime)); None
            }
            _ = cancellation.cancelled(), if stop.is_none() => None,
            joined = children.join_next_with_id() => joined,
        };
        let Some(joined) = joined else { continue };
        let (id, result) = match joined {
            Ok((id, result)) => (id, result.map_err(ScheduleChildError::Run)),
            Err(error) => {
                settled = false;
                (error.id(), Err(ScheduleChildError::Join(error)))
            }
        };
        let Some(index) = active.remove(&id) else {
            settled = false;
            stop = Some(ScheduleStop::Unsettled);
            cancellation.cancel_with_reason(CancellationReason::OwnerFailure);
            continue;
        };
        let result = match result {
            Ok(output) if output.truncated => Err(ScheduleChildError::Incomplete),
            other => other,
        };
        states[index] = if result.is_ok() {
            NodeState::Succeeded
        } else {
            NodeState::Failed
        };
        if result.is_err() && stop.is_none() {
            stop = Some(ScheduleStop::ChildFailed);
            cancellation.cancel_with_reason(CancellationReason::OwnerFailure);
        }
        outcomes.push(ScheduleOutcome {
            agent_id: plans[index].agent.clone(),
            invocation_id: plans[index].invocation.clone(),
            result,
        });
    }
    // Returning from every future is necessary; unsettled slot charges still
    // prevent successful release, including a failed finish_child operation.
    let final_budget = ledger.snapshot().await;
    if settled && final_budget.child_slots_active == 0 {
        lease.complete();
    } else {
        stop = Some(ScheduleStop::Unsettled);
    }
    // Preserve a stronger owner failure that latched while children were joining.
    if (final_budget.poisoned || owner.reason() == Some(CancellationReason::OwnerFailure))
        && stop != Some(ScheduleStop::Unsettled)
    {
        stop = Some(ScheduleStop::OwnerFailure);
    }
    Ok(ScheduleReport {
        outcomes,
        stop,
        not_started: plans
            .iter()
            .zip(states)
            .filter(|(_, state)| *state == NodeState::Pending)
            .map(|(plan, _)| plan.agent.clone())
            .collect(),
    })
}
