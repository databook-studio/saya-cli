use crate::{
    RenderFormat, TerminalEvent, answering_receipts::AnsweringReceipts,
    interactive::turn_summary::TallyHandle, render::Rendered, render_event,
};
use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink};
use std::{
    io::{self, Write},
    sync::Mutex,
};

include!("stream_render/sink.rs");
include!("stream_render/text_groups.rs");
include!("stream_render/event_map.rs");
#[cfg(test)]
mod tests {
    use super::*;
    include!("stream_render/tests/setup.rs");
    include!("stream_render/tests/memory_events.rs");
    include!("stream_render/tests/stream_basics.rs");
    include!("stream_render/tests/learning_events.rs");
    include!("stream_render/tests/progress_and_consensus.rs");
    include!("stream_render/tests/usage_events.rs");
    include!("stream_render/tests/recovery_events.rs");
    include!("stream_render/tests/group_helpers.rs");
    include!("stream_render/tests/group_streams.rs");
    include!("stream_render/tests/boundaries.rs");
    include!("stream_render/tests/remaining_contracts.rs");
}
