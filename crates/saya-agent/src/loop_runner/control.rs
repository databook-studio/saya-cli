use super::AgentLimits;
use crate::{
    AgentEventSink, AgentRequest, ApprovalDecider, CancellationToken, ChatProvider, ToolDefinition,
    ToolExecutor,
};

/// Optional ceilings shared by every logical answering request in a run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnsweringLimits {
    pub max_requests: Option<u64>,
    pub max_tool_calls: Option<u64>,
    pub max_reported_tokens: Option<u64>,
}

/// Inputs for the controlled answering API, grouped to keep its call surface small.
pub struct AgentRunControl<'a> {
    pub(super) limits: AgentLimits,
    pub(super) answering: AnsweringLimits,
    pub(super) approval: &'a dyn ApprovalDecider,
    pub(super) sink: &'a dyn AgentEventSink,
    pub(super) cancellation: CancellationToken,
}

impl<'a> AgentRunControl<'a> {
    pub fn new(
        limits: AgentLimits,
        answering: AnsweringLimits,
        approval: &'a dyn ApprovalDecider,
        sink: &'a dyn AgentEventSink,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            limits,
            answering,
            approval,
            sink,
            cancellation,
        }
    }
}

pub(super) struct AgentRunInvocation<'a> {
    pub(super) provider: &'a dyn ChatProvider,
    pub(super) tools: &'a dyn ToolExecutor,
    pub(super) request: AgentRequest,
    pub(super) definitions: Vec<ToolDefinition>,
    pub(super) limits: AgentLimits,
    pub(super) approval: &'a dyn ApprovalDecider,
    pub(super) sink: &'a dyn AgentEventSink,
    pub(super) cancellation: CancellationToken,
    pub(super) answering: Option<AnsweringLimits>,
}
