use super::knowledge_event::knowledge_supplied_event;
use super::limits::{AnsweringSurface, answering_deadline, answering_limits};
use super::provider::provider_message;
use super::question_scope::QuestionScope;
use super::tools;
use super::turn_config::{
    AgentRuntimeError, PromptOverrides, query_data_allowed_for_endpoint, with_next_step,
};
use super::turn_inputs::{TurnInputs, prepare_turn};
use crate::agent::hooks::{HookApprovalGate, HookCaller, HookGate, Hooks};
use crate::agent::tools::consent::{OwnApproval, OwnedToolApproval};
use crate::interactive::session::universe::SessionUniverse;
use crate::{
    config::runtime::RuntimeConfig, grant_token::TurnPrimary, prompt_approval::TerminalApproval,
};
use saya_agent::{
    AgentError, AgentEvent, AgentEventSink, AgentLimits, AgentMode, AgentOutput, AgentRequest,
    AgentRunControl, ApprovalDecider, ApprovalPolicy, BudgetDimension, CancellationReason,
    CancellationToken, ChatMessage, LocalStateEffect, run_agent_with_control,
};
use saya_store::SqliteStateStore;
use std::sync::Arc;

pub(crate) struct SinkPrompt<'a> {
    pub(crate) runtime: &'a RuntimeConfig,
    pub(crate) prompt: &'a str,
    pub(crate) approval: ApprovalPolicy,
    pub(crate) can_prompt: bool,
    pub(crate) can_obtain_approval: bool,
    pub(crate) overrides: PromptOverrides,
    pub(crate) history: Vec<ChatMessage>,
    pub(crate) sink: &'a dyn AgentEventSink,
    pub(crate) cancellation: CancellationToken,
    /// Present when an outer question owns the budget across several runtime
    /// invocations (candidate attempts or a clarification continuation).
    pub(crate) question_scope: Option<QuestionScope>,
    pub(crate) state_db: Option<SqliteStateStore>,
    pub(crate) decider: Option<Arc<dyn ApprovalDecider>>,
    pub(crate) last_sql: Option<String>,
    pub(crate) session: Option<Arc<SessionUniverse>>,
    pub(crate) agent_mode: AgentMode,
    pub(crate) capture: Option<tools::CaptureHook>,
    pub(crate) prior_tool_outcomes:
        Option<crate::interactive::session::continuation::PriorToolOutcomes>,
    pub(crate) continuation: Option<saya_store::StoredContinuation>,
    pub(crate) compaction_summary: Option<String>,
}

pub(crate) struct TurnPrompt<'a> {
    pub(crate) runtime: &'a RuntimeConfig,
    pub(crate) inputs: TurnInputs,
    pub(crate) prompt: &'a str,
    pub(crate) approval: ApprovalPolicy,
    pub(crate) can_prompt: bool,
    pub(crate) can_obtain_approval: bool,
    pub(crate) history: Vec<ChatMessage>,
    pub(crate) sink: &'a dyn AgentEventSink,
    pub(crate) cancellation: CancellationToken,
    pub(crate) question_scope: Option<QuestionScope>,
    pub(crate) state_db: Option<SqliteStateStore>,
    pub(crate) decider: Option<Arc<dyn ApprovalDecider>>,
    pub(crate) last_sql: Option<String>,
    pub(crate) session: Option<Arc<SessionUniverse>>,
    pub(crate) agent_mode: AgentMode,
    pub(crate) capture: Option<tools::CaptureHook>,
    pub(crate) prior_tool_outcomes:
        Option<crate::interactive::session::continuation::PriorToolOutcomes>,
    pub(crate) continuation: Option<saya_store::StoredContinuation>,
    pub(crate) compaction_summary: Option<String>,
}

include!("runtime/sink_outcomes.rs");
include!("runtime/question_start.rs");
include!("runtime/injected_inputs.rs");
include!("runtime/turn.rs");
include!("runtime/project_context.rs");
include!("runtime/session_context.rs");
include!("runtime/execute_turn.rs");
include!("runtime/finish_turn.rs");

mod ask_surface;

#[cfg(test)]
#[path = "runtime/ask_surface_tests.rs"]
mod ask_surface_tests;

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "runtime_budget_tests.rs"]
mod budget_tests;
