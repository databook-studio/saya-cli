use super::knowledge_event::knowledge_supplied_event;
use super::question_scope::QuestionScope;
use super::tools;
#[cfg(test)]
pub(crate) use super::turn_config::query_data_allowed;
pub(crate) use super::turn_config::{
    AgentRuntimeError, PromptOverrides, effective_ai, query_data_allowed_for_endpoint,
};
use super::turn_inputs::{TurnInputs, prepare_turn};
use crate::interactive::session::universe::SessionUniverse;
use crate::{
    config::runtime::RuntimeConfig, grant_token::TurnPrimary, prompt_approval::TerminalApproval,
};
use saya_agent::{
    AgentError, AgentEvent, AgentEventSink, AgentLimits, AgentMode, AgentOutput, AgentRequest,
    AgentRunControl, AnsweringLimits, ApprovalDecider, ApprovalPolicy, BudgetDimension,
    CancellationReason, CancellationToken, ChatMessage, LocalStateEffect, run_agent_with_control,
};
use saya_config::BudgetLimit;
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

include!("runtime/entrypoint.rs");
include!("runtime/sink_outcomes.rs");
include!("runtime/question_budget.rs");
include!("runtime/question_start.rs");
include!("runtime/injected_inputs.rs");
include!("runtime/turn.rs");
include!("runtime/project_context.rs");
include!("runtime/session_context.rs");
include!("runtime/execute_turn.rs");
include!("runtime/finish_turn.rs");

fn finite_limit(limit: BudgetLimit) -> Option<u64> {
    match limit {
        BudgetLimit::Finite(value) => Some(value.get()),
        BudgetLimit::Unlimited => None,
    }
}

/// the three first-run failures used to name no next step. Guidance is
/// added at the saya-cli boundary (here), not in the provider-neutral agent
/// crate. A provider that is configured but unreachable is "what is configured
/// did not work", not "nothing is configured" — so the next step is to start
/// the provider or check the endpoint, never to re-run `config init` (telling a
/// user whose gateway is momentarily down to re-init would be worse than
/// saying nothing).
fn provider_message(error: &saya_agent::ProviderError) -> String {
    let message = error.to_string();
    if message.contains("could not reach the provider") {
        format!(
            "{message}\nnext: start your AI provider, or run `saya config doctor` to check \
             the endpoint and base_url."
        )
    } else {
        message
    }
}

/// an unresolvable secret reference is "what is configured did not
/// work" — the profile is there but its password is not. `init` cannot supply
/// a secret, so the next step is setting the env var (doctor lists the
/// unresolved references). Only the secret case is annotated; a genuine
/// connection failure is left untouched rather than given advice that might
/// not fit. Applied at the `prepare_turn` seam so both the `ask` and the
/// interactive paths surface it.
fn with_next_step(error: AgentRuntimeError) -> AgentRuntimeError {
    match error {
        AgentRuntimeError::Database(message)
            if message.contains("secret reference")
                && message.contains("could not be resolved") =>
        {
            AgentRuntimeError::Database(format!(
                "{message}\nnext: set the referenced environment variable (or add it to a \
                 .env.saya file passed with --env-file), then re-run; `saya config doctor` \
                 lists unresolved secrets."
            ))
        }
        other => other,
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "runtime_budget_tests.rs"]
mod budget_tests;
