//! Pure graph validation and active lease admission for one finite schedule.
use super::super::integration::AssignedPaths;
use super::schedule_types::{MAX_SCHEDULE_TASKS, ScheduleError, ScheduleLimits, ScheduledChild};
use saya_types::{AgentId, InvocationId, JobId};
use std::collections::{HashMap, HashSet};

pub(super) struct NodePlan {
    pub agent: AgentId,
    pub invocation: InvocationId,
    pub dependencies: Vec<usize>,
    pub paths: Option<AssignedPaths>,
    pub provider: String,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum NodeState {
    Pending,
    Running,
    Succeeded,
    Failed,
}

pub(super) fn validate(
    nodes: &[ScheduledChild],
    job: &JobId,
) -> Result<Vec<NodePlan>, ScheduleError> {
    if nodes.is_empty() || nodes.len() > MAX_SCHEDULE_TASKS {
        return Err(ScheduleError::InvalidBatch);
    }
    let first = nodes[0].spec.authority();
    let mut ids = HashMap::new();
    for (index, node) in nodes.iter().enumerate() {
        let view = node.spec.authority();
        if view.job_id() != job
            || view.repository_id() != first.repository_id()
            || view.snapshot_id() != first.snapshot_id()
            || view.policy_digest() != first.policy_digest()
            || view.revocation_epoch() != first.revocation_epoch()
        {
            return Err(ScheduleError::IdentityMismatch);
        }
        if ids.insert(view.agent_id().clone(), index).is_some() {
            return Err(ScheduleError::InvalidBatch);
        }
    }
    let mut plans = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        let mut seen = HashSet::new();
        let mut dependencies = Vec::with_capacity(node.dependencies.len());
        for dependency in &node.dependencies {
            let Some(&target) = ids.get(dependency) else {
                return Err(ScheduleError::InvalidDependencies);
            };
            if target == index || !seen.insert(target) {
                return Err(ScheduleError::InvalidDependencies);
            }
            dependencies.push(target);
        }
        plans.push(NodePlan {
            agent: node.spec.authority().agent_id().clone(),
            invocation: node.spec.authority().invocation_id().clone(),
            dependencies,
            paths: node.paths.clone(),
            provider: node.provider.clone(),
        });
    }
    let mut resolved = vec![false; plans.len()];
    for _ in 0..plans.len() {
        let next = plans.iter().enumerate().find(|(index, node)| {
            !resolved[*index]
                && node
                    .dependencies
                    .iter()
                    .all(|dependency| resolved[*dependency])
        });
        let Some((index, _)) = next else {
            return Err(ScheduleError::Cycle);
        };
        resolved[index] = true;
    }
    Ok(plans)
}

pub(super) fn can_start(
    index: usize,
    plans: &[NodePlan],
    states: &[NodeState],
    limits: ScheduleLimits,
) -> bool {
    let node = &plans[index];
    if states[index] != NodeState::Pending
        || node
            .dependencies
            .iter()
            .any(|dependency| states[*dependency] != NodeState::Succeeded)
    {
        return false;
    }
    let active = states
        .iter()
        .enumerate()
        .filter(|(_, state)| **state == NodeState::Running)
        .map(|(index, _)| &plans[index])
        .collect::<Vec<_>>();
    active.len() < limits.max_active_children
        && active
            .iter()
            .filter(|other| other.provider == node.provider)
            .count()
            < limits.max_active_per_provider
        && !active
            .iter()
            .any(|other| match (&node.paths, &other.paths) {
                (Some(paths), Some(other)) => paths.overlaps(other),
                _ => false,
            })
}
