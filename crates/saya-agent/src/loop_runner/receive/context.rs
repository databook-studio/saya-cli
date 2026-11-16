use super::super::{admission::Admission, preflight::RequestPreflightConfig};
use crate::{
    ActorCorrelationDigest, AgentEvent, AgentEventSink, CancellationToken, ChatMessage,
    ChatProvider, CorrelationId, EffectAdmission, EffectCallKind, ToolDefinition,
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
    pub(in crate::loop_runner) checkpoint: &'a super::super::coding_checkpoint::CodingCheckpoint,
    pub(in crate::loop_runner) messages: &'a mut Vec<ChatMessage>,
    pub(in crate::loop_runner) ordinary_history_pairs: &'a mut usize,
    pub(in crate::loop_runner) protected_prefix_len: &'a mut usize,
    pub(in crate::loop_runner) definitions: &'a [ToolDefinition],
    pub(in crate::loop_runner) context_byte_budget: usize,
    pub(in crate::loop_runner) request_preflight: Option<RequestPreflightConfig>,
    pub(in crate::loop_runner) continuation: Option<ContinuationProgress>,
    /// The effort the loop's policy chose for this request. A configured
    /// effort arrives here already pinned.
    pub(in crate::loop_runner) reasoning_effort: crate::ReasoningEffort,
    /// What remains of the job's wall-clock ceiling, when it has one. The
    /// per-request deadline is the smaller of this and the provider's cap.
    pub(in crate::loop_runner) remaining_wall: Option<std::time::Duration>,
    pub(in crate::loop_runner) sink: &'a dyn AgentEventSink,
    pub(in crate::loop_runner) cancellation: &'a CancellationToken,
    pub(in crate::loop_runner) events: &'a mut Vec<AgentEvent>,
    pub(in crate::loop_runner) admission: &'a mut Admission,
    pub(in crate::loop_runner) effect_admission: Option<&'a dyn EffectAdmission>,
    pub(in crate::loop_runner) require_physical_effect_admission: bool,
    pub(in crate::loop_runner) call_kind: EffectCallKind,
    pub(in crate::loop_runner) conversation_correlation: CorrelationId,
    pub(in crate::loop_runner) invocation_correlation: CorrelationId,
    pub(in crate::loop_runner) actor_correlation: Option<ActorCorrelationDigest>,
    pub(in crate::loop_runner) request_ordinal: u64,
}
