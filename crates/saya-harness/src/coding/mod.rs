//! Native coding coordination contracts.

pub mod budget;
pub mod context;
pub mod policy;

pub mod child {
    mod factory;
    pub(crate) mod failure;
    mod mailbox;
    mod spec;
    pub use mailbox::ChildControl;

    pub use factory::{ChildComponents, ChildFactory, ChildFactoryError, OwnerFailureSnapshot};
    pub use spec::{
        ChildExecutionSpec, ChildSpecError, MAX_CHILD_FOLLOW_UP_CONTEXT_BYTES,
        MAX_CHILD_PROFILE_NAMES, MAX_CHILD_REQUEST_BYTES, MAX_CHILD_REQUEST_ITEMS,
        MAX_CHILD_REQUEST_MODEL_CHARS,
    };
}

pub mod coordinator {
    mod admit;
    mod cancel;
    mod recovery;
    mod resume;
    pub use recovery::inspect_recovery;
    pub use resume::{ResumeError, resume_job, resume_uncapped_job};
    mod reconcile;
    pub use reconcile::{
        ReconcileError, ReconcileReport, ReconciliationPlan, UnresolvedEffect, plan_reconciliation,
        reconcile_effects,
    };
    mod failure_class;
    pub use failure_class::{
        agent_class as classify_agent_failure, classify as classify_child_failure,
    };
    mod notify;
    mod results;
    pub use cancel::CHILD_CANCEL_JOIN_GRACE;
    pub use notify::{acknowledge_completion, replay_completions};
    pub use results::CompletionError;
    mod api;
    mod event_projection;
    mod events;
    mod schedule;
    mod schedule_types;
    mod scheduled_child;
    mod session;
    mod session_loop;

    pub use schedule::run_schedule;
    pub use schedule_types::{
        ChildOutcome, MAX_SCHEDULE_DEPENDENCIES, MAX_SCHEDULE_EVENT_CAPACITY, MAX_SCHEDULE_TASKS,
        ScheduleChildError, ScheduleError, ScheduleLimits, ScheduleOutcome, ScheduleReport,
        ScheduleStop, ScheduledChild,
    };
    pub use session::{ScheduleSession, SessionRun};

    pub use api::{ChildCleanupFailure, ChildRunError, run_child, run_child_budgeted};
    pub use events::{
        ChildEvent, ChildEventChannelError, ChildEventKind, ChildEventReceiver, ChildEventSender,
        ChildMetric, ChildProgress, MAX_CHILD_EVENT_BYTES, MAX_CHILD_EVENT_QUEUE_CAPACITY,
        child_event_channel,
    };
}

pub mod git;
pub mod integration;
pub mod local;
pub mod snapshot;
pub mod worktree;

pub mod extensions;
