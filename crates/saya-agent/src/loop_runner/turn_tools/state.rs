//! Shared per-turn state for tool execution: the immutable inputs every gate
//! consults, and the mutable records the batch and serial paths append to.

use super::super::failed_statements;
use crate::{
    AgentEvent, AgentEventSink, AgentLimits, ApprovalDecider, CancellationToken, ChatMessage,
    ToolDefinition, ToolExecutor,
};

pub(super) struct TurnState<'a> {
    pub(super) tools: &'a dyn ToolExecutor,
    pub(super) definitions: &'a [ToolDefinition],
    pub(super) result_caps: &'a [usize],
    pub(super) limits: &'a AgentLimits,
    pub(super) approval: &'a dyn ApprovalDecider,
    pub(super) sink: &'a dyn AgentEventSink,
    pub(super) cancellation: &'a CancellationToken,
}

pub(super) struct TurnOutputs<'a> {
    pub(super) events: &'a mut Vec<AgentEvent>,
    pub(super) messages: &'a mut Vec<ChatMessage>,
    pub(super) used_bounded_sql_query: &'a mut bool,
    pub(super) tool_metadata: &'a mut Vec<crate::ToolMetadata>,
    pub(super) failed: &'a mut failed_statements::FailedStatements,
    pub(super) last_successful_sql: &'a mut Option<String>,
}
