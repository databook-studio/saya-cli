//! Native coding coordination contracts.

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

pub mod coordinator {
    mod api;
    mod event_projection;

    pub use api::{
        ChildEvent, ChildEventChannelError, ChildEventKind, ChildEventReceiver, ChildEventSender,
        ChildMetric, ChildProgress, ChildRunError, MAX_CHILD_EVENT_BYTES,
        MAX_CHILD_EVENT_QUEUE_CAPACITY, child_event_channel, run_child,
    };
}
