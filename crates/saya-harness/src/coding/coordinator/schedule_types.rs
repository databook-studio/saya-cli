//! Bounded one-shot schedule data, not authority or durable job state.
use super::super::{
    child::{ChildExecutionSpec, ChildFactory},
    integration::AssignedPaths,
};
use super::api::ChildRunError;
use saya_agent::{AgentOutput, CancellationReason};
use saya_types::{AgentId, InvocationId, MAX_NAME_CHARS, is_name_shaped};
use std::sync::Arc;

pub const MAX_SCHEDULE_TASKS: usize = 32;
pub const MAX_SCHEDULE_DEPENDENCIES: usize = 8;
pub const MAX_SCHEDULE_EVENT_CAPACITY: usize = 128;

pub struct ScheduledChild {
    pub(super) control: super::super::child::ChildControl,
    pub(super) mailbox: saya_agent::AgentMailbox,
    pub(super) spec: ChildExecutionSpec,
    pub(super) dependencies: Vec<AgentId>,
    pub(super) paths: Option<AssignedPaths>,
    pub(super) provider: String,
    pub(super) factory: Arc<dyn ChildFactory>,
}
impl ScheduledChild {
    pub fn control(&self) -> super::super::child::ChildControl {
        self.control.clone()
    }
    /// The trusted caller binds provider identity to this exact factory and routes
    /// all job children through the scheduler. Lease paths grant no write access.
    pub fn new(
        spec: ChildExecutionSpec,
        dependencies: Vec<AgentId>,
        paths: Option<AssignedPaths>,
        provider: String,
        factory: Arc<dyn ChildFactory>,
    ) -> Result<Self, ScheduleError> {
        if dependencies.len() > MAX_SCHEDULE_DEPENDENCIES {
            return Err(ScheduleError::InvalidDependencies);
        }
        if provider.chars().count() > MAX_NAME_CHARS || !is_name_shaped(&provider) {
            return Err(ScheduleError::InvalidProvider);
        }
        if spec.authority().write_paths().iter().any(|path| {
            !paths
                .as_ref()
                .is_some_and(|paths| paths.contains(path.as_str()))
        }) {
            return Err(ScheduleError::AssignmentMismatch);
        }
        let (control, mailbox) = super::super::child::ChildControl::new();
        Ok(Self {
            control,
            mailbox,
            spec,
            dependencies,
            paths,
            provider,
            factory,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ScheduleLimits {
    pub max_active_children: usize,
    pub max_active_per_provider: usize,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ScheduleError {
    #[error("job runtime needs matching fresh root authority before admission")]
    FreshAuthorityRequired,
    #[error("schedule must contain one through 32 distinct children")]
    InvalidBatch,
    #[error("schedule dependencies are duplicated, unknown or self-referential")]
    InvalidDependencies,
    #[error("schedule contains a dependency cycle")]
    Cycle,
    #[error("schedule identities do not share the same job, repository, snapshot and policy")]
    IdentityMismatch,
    #[error("schedule assignment does not cover granted write paths")]
    AssignmentMismatch,
    #[error("schedule provider key is not a bounded safe name")]
    InvalidProvider,
    #[error("schedule limits exceed job capacity or root event bounds")]
    InvalidLimits,
    #[error("another schedule owns this job ledger")]
    Busy,
    #[error("the job already has active child work outside this schedule")]
    ActiveChildren,
    #[error("the job budget owner is unavailable")]
    OwnerUnavailable,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ScheduleChildError {
    #[error("child returned bounded-effort output rather than completing")]
    Incomplete,
    #[error(transparent)]
    Run(#[from] ChildRunError),
    #[error("scheduled child task failed to join")]
    Join(#[source] tokio::task::JoinError),
}
#[derive(Debug)]
pub struct ScheduleOutcome {
    pub agent_id: AgentId,
    pub invocation_id: InvocationId,
    pub result: Result<AgentOutput, ScheduleChildError>,
    /// Durable notification identity; None means completion could not be persisted.
    pub completion_sequence: Option<std::num::NonZeroU64>,
    pub completion_error: Option<super::results::CompletionError>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleStop {
    /// The shared budget owner failed, even if the caller also cancelled.
    OwnerFailure,
    ChildFailed,
    Cancelled(CancellationReason),
    DependencyBlocked,
    Unsettled,
}
#[derive(Debug)]
pub struct ScheduleReport {
    /// Local joins can finish while remote effects retain durable unknown charges.
    pub effects_uncertain: bool,
    pub outcomes: Vec<ScheduleOutcome>,
    pub not_started: Vec<AgentId>,
    pub stop: Option<ScheduleStop>,
}
