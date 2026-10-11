//! Native coding coordination contracts.

pub mod budget;
pub mod context;
pub mod policy;
pub mod skills;

pub mod child {
    mod factory;

    pub use factory::{
        ChildComponents, ChildExecutionSpec, ChildFactory, ChildFactoryError, ChildSpecError,
        MAX_CHILD_PROFILE_NAMES, MAX_CHILD_REQUEST_BYTES, MAX_CHILD_REQUEST_ITEMS,
        MAX_CHILD_REQUEST_MODEL_CHARS,
    };
}

pub mod checks;
pub mod commands;

pub mod coordinator {
    mod api;
    mod event_projection;
    mod events;

    pub use api::{ChildRunError, run_child, run_child_budgeted};
    pub use events::{
        ChildEvent, ChildEventChannelError, ChildEventKind, ChildEventReceiver, ChildEventSender,
        ChildMetric, ChildProgress, MAX_CHILD_EVENT_BYTES, MAX_CHILD_EVENT_QUEUE_CAPACITY,
        child_event_channel,
    };
}

pub mod git;
pub mod integration;
pub mod snapshot;
pub mod worktree;
