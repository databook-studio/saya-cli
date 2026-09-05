mod designation;
mod failed_statements;
mod output;
mod receive;
mod salvage;
mod tools;
mod turn_tools;

use crate::{
    AgentEvent, AgentEventSink, AgentRequest, ApprovalDecider, CancellationToken, ChatProvider,
    TokenUsage, ToolDefinition, ToolExecutor,
};

pub use output::{AgentError, AgentLimits, AgentOutput, DESIGNATE_ANSWER_TOOL, budgets_from_env};

/// Add a turn's optional count into a run total without inventing data.
///
/// A turn that reported nothing leaves the total untouched, and the total stays
/// `None` until some turn reports — so a provider that never reports cache reads
/// stays distinguishable from one reporting a cold cache. Folding `None` in as
/// zero would collapse that distinction and make an unreported rate render as 0%.
fn sum_reported(total: &mut Option<u64>, turn: Option<u64>) {
    if let Some(count) = turn {
        *total = Some(total.unwrap_or(0) + count);
    }
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
    let mut messages = crate::history::build_messages(
        request.system_prompt.as_deref(),
        &request.context_blocks,
        &request.prompt,
        &request.history,
        limits.context_byte_budget,
    )?;
    let mut events = Vec::new();
    let mut tool_count = 0;
    let mut used_bounded_sql_query = false;
    let mut tool_metadata = Vec::new();
    let mut usage = TokenUsage::default();
    let mut turn_count = 0;
    // Statements that failed during this run, so a byte-identical
    // re-submission is refused rather than re-executed (loop invariant for the
    // "do not repeat a failed query" advice the model does not always obey).
    let mut failed = failed_statements::FailedStatements::new();
    // The last statement that completed successfully, so a run that exhausts its
    // budget without nominating can still surface its best available answer.
    let mut last_successful_sql: Option<String> = None;
    loop {
        check_cancelled(&cancellation)?;
        if let Some(max_turns) = limits.max_turns
            && turn_count >= max_turns
        {
            return salvage::salvage(
                provider,
                &request.model,
                &mut messages,
                &[],
                limits.context_byte_budget,
                sink,
                &cancellation,
                &mut events,
                &mut usage,
                used_bounded_sql_query,
                tool_metadata,
                last_successful_sql,
            )
            .await;
        }
        turn_count += 1;
        let (assistant, turn_usage, reasoning) = receive::receive(
            provider,
            &request.model,
            &messages,
            &definitions,
            sink,
            &cancellation,
            &mut events,
        )
        .await?;
        // Providers report cumulative counts per response; sum across turns.
        usage.input_tokens += turn_usage.input_tokens;
        usage.output_tokens += turn_usage.output_tokens;
        sum_reported(
            &mut usage.cached_input_tokens,
            turn_usage.cached_input_tokens,
        );
        sum_reported(
            &mut usage.cache_creation_input_tokens,
            turn_usage.cache_creation_input_tokens,
        );
        sum_reported(&mut usage.reasoning_tokens, turn_usage.reasoning_tokens);
        // Forward the turn's captured chain-of-thought onto the event stream as
        // one `ReasoningText` event — the provider layer puts it on `ChatResponse.reasoning`
        // and bound it to `_reasoning` here; the CLI-boundary slice carries it across the crate
        // boundary so the CLI *can* reach it; whether to display it is the CLI's call.
        // This is the only way reasoning leaves `saya-agent`: it is NOT pushed
        // onto `messages` — `assistant` (a `ChatMessage`) is what gets replayed
        // to the provider as history, and `ChatMessage` has no reasoning field,
        // so reasoning cannot leak into the next turn's request. `None` (a
        // provider that reported no reasoning) emits nothing — byte-identical
        // to today.
        if let Some(text) = reasoning
            && !text.is_empty()
        {
            emit(&mut events, sink, AgentEvent::reasoning_text(text)).await;
        }
        messages.push(assistant.clone());
        // The model designates the SQL that answers the question by calling
        // `designate_answer` in its terminal turn, alongside the prose answer.
        // That ends the run: the prose is the answer, the SQL is carried on
        // the output and an event, and no tool is executed. Optional — a turn
        // without the call falls through to the normal terminal below.
        if let Some(sql) = designation::designation_from(&assistant) {
            emit(
                &mut events,
                sink,
                AgentEvent::answer_designated(sql.clone()),
            )
            .await;
            check_cancelled(&cancellation)?;
            emit(&mut events, sink, AgentEvent::Complete).await;
            return Ok(AgentOutput {
                answer: assistant.content,
                events,
                used_bounded_sql_query,
                tool_metadata,
                usage,
                learning_usage: None,
                truncated: false,
                answer_sql: Some(sql),
            });
        }
        if assistant.tool_calls.is_empty() {
            check_cancelled(&cancellation)?;
            emit(&mut events, sink, AgentEvent::Complete).await;
            return Ok(AgentOutput {
                answer: assistant.content,
                events,
                used_bounded_sql_query,
                tool_metadata,
                usage,
                learning_usage: None,
                truncated: false,
                answer_sql: None,
            });
        }
        // The tool-call ceiling is a whole-run total checked once per turn,
        // before any of the turn's calls run: if this turn would cross it, the
        // run stops rather than executing a partial batch.
        let projected_tool_calls = tool_count + assistant.tool_calls.len();
        if let Some(max_tool_calls) = limits.max_tool_calls
            && projected_tool_calls > max_tool_calls
        {
            return salvage::salvage(
                provider,
                &request.model,
                &mut messages,
                &assistant.tool_calls,
                limits.context_byte_budget,
                sink,
                &cancellation,
                &mut events,
                &mut usage,
                used_bounded_sql_query,
                tool_metadata,
                last_successful_sql,
            )
            .await;
        }
        tool_count = projected_tool_calls;
        let batch_ran = turn_tools::run_turn_tools(
            tools,
            assistant,
            &definitions,
            &limits,
            approval,
            sink,
            &cancellation,
            &mut events,
            &mut messages,
            &mut used_bounded_sql_query,
            &mut tool_metadata,
            &mut failed,
            &mut last_successful_sql,
        )
        .await?;
        if batch_ran {
            continue;
        }
        // Intra-loop context budget: the pre-loop trim bounds history, but
        // assistant turns and tool results accumulate here. Trim the oldest
        // tool-result pairs (the assistant turn that issued each call plus its
        // `tool` message) until the conversation fits, keeping the newest
        // context — the same recency policy the pre-loop path uses. A single
        // result is already capped at construction, so this resolves
        // accumulation; if trimming everything still leaves the newest result
        // over budget, truncate it rather than aborting the whole run.
        output::trim_to_budget(&mut messages, limits.context_byte_budget);
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
fn check_cancelled(token: &CancellationToken) -> Result<(), AgentError> {
    if token.is_cancelled() {
        Err(AgentError::Cancelled)
    } else {
        Ok(())
    }
}
