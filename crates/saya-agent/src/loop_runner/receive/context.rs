use super::super::{admission::Admission, preflight::RequestPreflightConfig};
use crate::{
    AgentEvent, AgentEventSink, CancellationToken, ChatMessage, ChatProvider, ToolDefinition,
};

#[derive(Clone, Copy)]
pub(in crate::loop_runner) struct ContinuationProgress {
    pub(in crate::loop_runner) index: usize,
    pub(in crate::loop_runner) limit: usize,
}

/// Streams one provider turn and returns the assembled assistant message. A
/// failed attempt's latest reported cumulative usage remains billable even
/// though its content is discarded.
pub(in crate::loop_runner) struct ReceiveContext<'a> {
    pub(in crate::loop_runner) provider: &'a dyn ChatProvider,
    pub(in crate::loop_runner) model: &'a str,
    pub(in crate::loop_runner) messages: &'a mut Vec<ChatMessage>,
    pub(in crate::loop_runner) ordinary_history_pairs: &'a mut usize,
    pub(in crate::loop_runner) protected_prefix_len: &'a mut usize,
    pub(in crate::loop_runner) definitions: &'a [ToolDefinition],
    pub(in crate::loop_runner) context_byte_budget: usize,
    pub(in crate::loop_runner) request_preflight: Option<RequestPreflightConfig>,
    pub(in crate::loop_runner) continuation: Option<ContinuationProgress>,
    pub(in crate::loop_runner) sink: &'a dyn AgentEventSink,
    pub(in crate::loop_runner) cancellation: &'a CancellationToken,
    pub(in crate::loop_runner) events: &'a mut Vec<AgentEvent>,
    pub(in crate::loop_runner) admission: &'a mut Admission,
}
