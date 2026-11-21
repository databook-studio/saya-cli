//! What one controlled answering call carries: grouped, so the loop's own
//! entry point stays a short list of arguments.
//!
//! `AgentRunControl` builds one of these per call; nothing else constructs one.

use super::{
    AgentLimits, AnsweringScope, DeferredAnsweringScope, ProgressHandle, RequestPreflightConfig,
    coding_checkpoint::AuthorityRefusalContext, steering::AgentMailbox,
};
use crate::{
    ActorCorrelationDigest, AgentEventSink, AgentRequest, ApprovalDecider, CancellationToken,
    ChatProvider, ContextPreamble, EffectAdmission, ToolDefinition, ToolExecutor,
};

pub(super) struct AgentRunInvocation<'a> {
    pub(super) provider: &'a dyn ChatProvider,
    pub(super) tools: &'a dyn ToolExecutor,
    pub(super) request: AgentRequest,
    pub(super) definitions: Vec<ToolDefinition>,
    pub(super) mailbox: Option<AgentMailbox>,
    pub(super) limits: AgentLimits,
    pub(super) approval: &'a dyn ApprovalDecider,
    pub(super) sink: &'a dyn AgentEventSink,
    pub(super) cancellation: CancellationToken,
    pub(super) answering: Option<AnsweringScope>,
    /// The turn's deferred budget, armed on its first plugin use, or `None` for
    /// a turn nothing may arm a plugin policy on.
    pub(super) deferred_budget: Option<DeferredAnsweringScope>,
    pub(super) request_preflight: Option<RequestPreflightConfig>,
    pub(super) effect_admission: Option<&'a dyn EffectAdmission>,
    pub(super) require_physical_effect_admission: bool,
    pub(super) actor_correlation: Option<ActorCorrelationDigest>,
    pub(super) authority_refusal_context: Option<AuthorityRefusalContext>,
    pub(super) context_preamble: ContextPreamble,
    pub(super) progress_report: Option<ProgressHandle>,
    pub(super) background_wait: Option<std::time::Duration>,
}
