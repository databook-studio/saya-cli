mod admission;
mod attempt;
mod attempt_metrics;
mod clarification;
mod clarification_args;
mod clarification_feed;
mod compaction;
mod control;
mod designation;
mod failed_statements;
mod failure_key;
mod output;
mod preflight;
mod receive;
mod receive_stream;
mod response;
mod response_tools;
mod runner;
mod salvage;
mod steering;
mod tool_batch;
mod tool_policy;
mod tool_protocol;
mod tool_record;
mod tool_shape;
mod tools;
mod turn;
mod turn_tools;

pub use clarification_args::{
    MAX_CLARIFICATION_OPTION_CHARS, MAX_CLARIFICATION_OPTIONS, MAX_CLARIFICATION_QUESTION_CHARS,
    REQUEST_CLARIFICATION_TOOL,
};

use crate::{
    AgentEvent, AgentEventSink, AgentRequest, ApprovalDecider, CancellationToken, ChatProvider,
    TokenUsage, ToolDefinition, ToolExecutor,
};

use control::AgentRunInvocation;
pub use control::{AgentRunControl, AnsweringLimits, AnsweringScope};
pub use output::{
    AgentError, AgentLimits, AgentOutput, DESIGNATE_ANSWER_TOOL, EnvBudgets, budgets_from_env,
};
pub use preflight::RequestPreflightConfig;
pub use steering::{
    AgentMailbox, AgentSteering, MAX_STEERING_MESSAGE_BYTES, MAX_STEERING_MESSAGES, SteeringError,
    agent_mailbox,
};
pub use tools::{MAX_TOOL_MESSAGE_BYTES, ShapedToolResult, shape_tool_result, tool_message_cap};

/// Re-instruction pushed as a user-role message after the provider caps a
/// response mid-answer. The incomplete response is discarded, never replayed:
/// replaying a half-assembled tool-argument JSON as history would invite the
/// model to complete a write whose first half it never issued. Committed work
/// needs no replay — `workspace_edit`'s append variant reports the file's size
/// and digest in its tool result, and those results are already in the
/// conversation, so the model resumes from there.
const CONTINUATION_NOTE: &str = "Your previous response was cut off at the provider's output-token limit. The incomplete part was discarded and is not in the conversation. If you were writing a file, resume it with the append variant of workspace_edit using the size and digest reported in your earlier tool result, do not restart the file. Emit less per response so the next one fits.";

/// Add a turn's optional count into a run total without inventing data.
///
/// A turn that reported nothing leaves the total untouched, and the total stays
/// `None` until some turn reports — so a provider that never reports cache reads
/// stays distinguishable from one reporting a cold cache. Folding `None` in as
/// zero would collapse that distinction and make an unreported rate render as 0%.
fn sum_reported(total: &mut Option<u64>, turn: Option<u64>) {
    if let Some(count) = turn {
        *total = Some(total.unwrap_or(0).saturating_add(count));
    }
}

pub(super) fn add_usage(total: &mut TokenUsage, turn: TokenUsage) {
    total.input_tokens = total.input_tokens.saturating_add(turn.input_tokens);
    total.output_tokens = total.output_tokens.saturating_add(turn.output_tokens);
    sum_reported(&mut total.cached_input_tokens, turn.cached_input_tokens);
    sum_reported(
        &mut total.cache_creation_input_tokens,
        turn.cache_creation_input_tokens,
    );
    sum_reported(&mut total.reasoning_tokens, turn.reasoning_tokens);
}

#[allow(clippy::too_many_arguments)]
pub async fn run_agent_with_sink(
    provider: &dyn ChatProvider,
    tools: &dyn ToolExecutor,
    request: AgentRequest,
    definitions: Vec<ToolDefinition>,
    limits: AgentLimits,
    approval: &dyn ApprovalDecider,
    sink: &dyn AgentEventSink,
    cancellation: CancellationToken,
) -> Result<AgentOutput, AgentError> {
    runner::run_agent_inner(AgentRunInvocation {
        provider,
        tools,
        request,
        definitions,
        limits,
        approval,
        sink,
        cancellation,
        mailbox: None,
        answering: None,
        request_preflight: None,
        effect_admission: None,
        require_physical_effect_admission: false,
    })
    .await
}

pub fn run_agent_with_control<'a>(
    provider: &'a dyn ChatProvider,
    tools: &'a dyn ToolExecutor,
    request: AgentRequest,
    definitions: Vec<ToolDefinition>,
    control: AgentRunControl<'a>,
) -> impl std::future::Future<Output = Result<AgentOutput, AgentError>> + Send + 'a {
    runner::run_agent_inner(AgentRunInvocation {
        provider,
        tools,
        request,
        definitions,
        mailbox: control.mailbox,
        limits: control.limits,
        approval: control.approval,
        sink: control.sink,
        cancellation: control.cancellation,
        answering: Some(control.answering_scope),
        request_preflight: control.request_preflight,
        effect_admission: control.effect_admission,
        require_physical_effect_admission: control.require_physical_effect_admission,
    })
}

pub(super) struct PartialOutput {
    pub(super) usage: TokenUsage,
    pub(super) used_bounded_sql_query: bool,
    pub(super) tool_metadata: Vec<crate::ToolMetadata>,
    pub(super) answer_sql: Option<String>,
    pub(super) answer: String,
}

pub(super) async fn budget_output(
    dimension: crate::BudgetDimension,
    events: &mut Vec<AgentEvent>,
    sink: &dyn AgentEventSink,
    partial: PartialOutput,
) -> AgentOutput {
    emit(
        events,
        sink,
        AgentEvent::answering_budget_stopped(dimension),
    )
    .await;
    AgentOutput {
        answer: partial.answer,
        events: std::mem::take(events),
        used_bounded_sql_query: partial.used_bounded_sql_query,
        tool_metadata: partial.tool_metadata,
        usage: partial.usage,
        learning_usage: None,
        truncated: true,
        answer_sql: partial.answer_sql,
    }
}

pub(super) async fn emit(
    events: &mut Vec<AgentEvent>,
    sink: &dyn AgentEventSink,
    event: AgentEvent,
) {
    sink.emit(event.clone()).await;
    events.push(event);
}

pub(super) fn check_cancelled(token: &CancellationToken) -> Result<(), AgentError> {
    if token.is_cancelled() {
        Err(AgentError::Cancelled)
    } else {
        Ok(())
    }
}
