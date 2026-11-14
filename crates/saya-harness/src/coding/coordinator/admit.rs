//! Pure graph validation and active-child admission for one finite schedule.
//!
//! Every child of a schedule shares the one working checkout, so admission
//! carries no per-path mutual exclusion: two children may write the same file,
//! and the shared tree — not this gate — owns that conflict.
use super::schedule_types::{MAX_SCHEDULE_TASKS, ScheduleError, ScheduleLimits, ScheduledChild};
use saya_types::{AgentId, InvocationId, JobId, RepositoryId, Sha256Digest, SnapshotId};
use std::collections::{HashMap, HashSet};

pub(super) struct NodePlan {
    pub agent: AgentId,
    pub invocation: InvocationId,
    pub dependencies: Vec<usize>,
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
    let identity = SessionIdentity::of(&nodes[0]);
    if &identity.job != job {
        return Err(ScheduleError::IdentityMismatch);
    }
    let mut ids = HashMap::new();
    for (index, node) in nodes.iter().enumerate() {
        if !identity.accepts(node) {
            return Err(ScheduleError::IdentityMismatch);
        }
        if ids
            .insert(node.spec.authority().agent_id().clone(), index)
            .is_some()
        {
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

/// The identity every child of one session must share: the authenticated job,
/// repository, snapshot, policy and revocation epoch of the session's first
/// child. A later submission is checked against it, never trusted.
pub(super) struct SessionIdentity {
    job: JobId,
    repository: RepositoryId,
    snapshot: SnapshotId,
    policy_digest: Sha256Digest,
    revocation_epoch: u64,
}

impl SessionIdentity {
    pub(super) fn of(node: &ScheduledChild) -> Self {
        let view = node.spec.authority();
        Self {
            job: view.job_id().clone(),
            repository: view.repository_id().clone(),
            snapshot: view.snapshot_id().clone(),
            policy_digest: view.policy_digest().clone(),
            revocation_epoch: view.revocation_epoch(),
        }
    }

    fn accepts(&self, node: &ScheduledChild) -> bool {
        let view = node.spec.authority();
        view.job_id() == &self.job
            && view.repository_id() == &self.repository
            && view.snapshot_id() == &self.snapshot
            && view.policy_digest() == &self.policy_digest
            && view.revocation_epoch() == self.revocation_epoch
    }
}

/// The growing index of one session's admitted children: the identity they all
/// share, the agent each was admitted as, and how many the session holds.
pub(super) struct SessionIndex {
    identity: SessionIdentity,
    agents: HashMap<AgentId, usize>,
    len: usize,
}

impl SessionIndex {
    pub(super) fn of(plans: &[NodePlan], identity: SessionIdentity) -> Self {
        Self {
            identity,
            agents: plans
                .iter()
                .enumerate()
                .map(|(index, plan)| (plan.agent.clone(), index))
                .collect(),
            len: plans.len(),
        }
    }

    /// Validates one later child against the session and resolves its
    /// dependencies to session indices. Nothing is recorded unless the whole
    /// child is admissible.
    pub(super) fn admit(&mut self, node: &ScheduledChild) -> Result<NodePlan, ScheduleError> {
        if !self.identity.accepts(node) {
            return Err(ScheduleError::IdentityMismatch);
        }
        if self.len >= MAX_SCHEDULE_TASKS {
            return Err(ScheduleError::InvalidBatch);
        }
        let agent = node.spec.authority().agent_id().clone();
        if self.agents.contains_key(&agent) {
            return Err(ScheduleError::InvalidBatch);
        }
        let index = self.len;
        let mut seen = HashSet::new();
        let mut dependencies = Vec::with_capacity(node.dependencies.len());
        for dependency in &node.dependencies {
            let Some(&target) = self.agents.get(dependency) else {
                return Err(ScheduleError::InvalidDependencies);
            };
            if target == index || !seen.insert(target) {
                return Err(ScheduleError::InvalidDependencies);
            }
            dependencies.push(target);
        }
        self.agents.insert(agent.clone(), index);
        self.len += 1;
        Ok(NodePlan {
            agent,
            invocation: node.spec.authority().invocation_id().clone(),
            dependencies,
            provider: node.provider.clone(),
        })
    }
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
}
