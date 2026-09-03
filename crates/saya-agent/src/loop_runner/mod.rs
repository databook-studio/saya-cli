mod output;
mod receive;
mod tools;

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
    loop {
        check_cancelled(&cancellation)?;
        if let Some(max_turns) = limits.max_turns
            && turn_count >= max_turns
        {
            return salvage(
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
        if let Some(sql) = designation_from(&assistant) {
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
            return salvage(
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
            )
            .await;
        }
        tool_count = projected_tool_calls;
        // When every call in the message is valid and auto-runnable, the
        // calls are independent: run them concurrently instead of paying
        // their latency sequentially. `auto_runnable` is the single policy
        // for "may this run with no questions asked"; the sequential path
        // below applies the same gates, so the two cannot drift.
        let batch_parallel = assistant.tool_calls.len() > 1
            && assistant.tool_calls.iter().all(|call| {
                tools::invalid_reason(call, &definitions).is_none()
                    && definitions
                        .iter()
                        .find(|definition| definition.name == call.name)
                        .is_some_and(|definition| tools::auto_runnable(definition, &limits))
            });
        if batch_parallel {
            check_cancelled(&cancellation)?;
            for call in &assistant.tool_calls {
                emit(
                    &mut events,
                    sink,
                    AgentEvent::ToolRequested {
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    },
                )
                .await;
                if definitions
                    .iter()
                    .find(|definition| definition.name == call.name)
                    .is_some_and(|definition| definition.effect.database_data)
                {
                    used_bounded_sql_query = true;
                }
            }
            let results =
                tools::execute_batch(tools, &assistant.tool_calls.clone(), &definitions).await;
            for (call, (result, summary)) in assistant.tool_calls.iter().zip(results) {
                let (message, truncated) =
                    tools::tool_message(call.id.clone(), result, limits.context_byte_budget);
                tool_metadata.push(crate::ToolMetadata {
                    name: call.name.clone(),
                    status: if summary.contains("failed") {
                        "failed"
                    } else {
                        "completed"
                    }
                    .into(),
                });
                messages.push(message);
                check_cancelled(&cancellation)?;
                emit(
                    &mut events,
                    sink,
                    AgentEvent::ToolCompleted {
                        name: call.name.clone(),
                        summary: output::completion_summary(summary, truncated),
                    },
                )
                .await;
            }
            continue;
        }
        for call in assistant.tool_calls {
            if let Some(reason) = tools::invalid_reason(&call, &definitions) {
                if call.id.trim().is_empty() {
                    return Err(AgentError::InvalidToolCall);
                }
                check_cancelled(&cancellation)?;
                emit(
                    &mut events,
                    sink,
                    AgentEvent::ToolRequested {
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    },
                )
                .await;
                // Feed the problem back as the tool result so the model can
                // retry with a valid call on its next turn.
                tool_metadata.push(crate::ToolMetadata {
                    name: call.name.clone(),
                    status: "failed".into(),
                });
                let (message, _) = tools::tool_message(
                    call.id,
                    serde_json::json!({"error": reason}),
                    limits.context_byte_budget,
                );
                messages.push(message);
                emit(
                    &mut events,
                    sink,
                    AgentEvent::ToolCompleted {
                        name: call.name,
                        summary: "tool call failed validation".into(),
                    },
                )
                .await;
                continue;
            }
            check_cancelled(&cancellation)?;
            emit(
                &mut events,
                sink,
                AgentEvent::ToolRequested {
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                },
            )
            .await;
            let definition = definitions
                .iter()
                .find(|tool| tool.name == call.name)
                .expect("validated");
            let approved = !definition.effect.requires_approval
                || approval.approve(definition, &call.arguments).await;
            // Apply the same policy the batch path consults (`auto_runnable`),
            // split into its gates so the denial can name which one refused.
            // `requires_approval` was already resolved into `approved`, so a
            // tool that needed approval and got it still runs; the remaining
            // gates bind whether or not approval was granted. This is the one
            // place the sequential path decides auto-run — keeping it here in
            // terms of the shared gates means a gate added to `tools.rs`
            // cannot apply to the batch path and not this one.
            let candidate_denied = tools::candidate_denied(definition, &limits);
            let side_effect_denied = tools::external_side_effect_gated(definition);
            let executed = approved && !candidate_denied && !side_effect_denied;
            let (result, summary) = if executed {
                check_cancelled(&cancellation)?;
                // Indicates a database-row-producing query tool ran.
                if definition.effect.database_data {
                    used_bounded_sql_query = true;
                }
                tools::execute(tools, &call.name, call.arguments, definition.read_only).await
            } else {
                emit(
                    &mut events,
                    sink,
                    AgentEvent::ToolDenied {
                        name: call.name.clone(),
                        reason: if side_effect_denied {
                            "external side effect requires approval".into()
                        } else if candidate_denied {
                            "candidate writes are not permitted".into()
                        } else {
                            "approval was not granted".into()
                        },
                    },
                )
                .await;
                (
                    serde_json::json!({"error":"tool call denied by approval policy"}),
                    "read-only database tool denied",
                )
            };
            tool_metadata.push(crate::ToolMetadata {
                name: call.name.clone(),
                status: if executed {
                    if summary.contains("failed") {
                        "failed"
                    } else {
                        "completed"
                    }
                } else {
                    "denied"
                }
                .into(),
            });
            let (message, truncated) =
                tools::tool_message(call.id, result, limits.context_byte_budget);
            messages.push(message);
            if executed {
                check_cancelled(&cancellation)?;
                emit(
                    &mut events,
                    sink,
                    AgentEvent::ToolCompleted {
                        name: call.name,
                        summary: output::completion_summary(summary, truncated),
                    },
                )
                .await;
            }
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

/// Appended to the conversation before the salvage call so the model knows its
/// tools are gone and it must answer from what it has, not call more tools.
const SALVAGE_INSTRUCTION: &str = "You have no further tool calls available. Using only the results you have already gathered, give your best answer to the question now.";

/// Salvages a run that has hit a ceiling instead of discarding everything. Any
/// tool calls the run did not get to execute are answered with a "budget
/// exhausted" result so the conversation the provider sees is well-formed;
/// then one final call with no tools asks for the best answer from the work
/// already done. The returned output is marked [`AgentOutput::truncated`]. If
/// the final call itself fails, its error is returned unchanged — the run does
/// not invent an answer and does not swallow the provider failure.
#[allow(clippy::too_many_arguments)]
async fn salvage(
    provider: &dyn ChatProvider,
    model: &str,
    messages: &mut Vec<crate::ChatMessage>,
    pending: &[crate::ToolCall],
    context_byte_budget: usize,
    sink: &dyn AgentEventSink,
    cancellation: &CancellationToken,
    events: &mut Vec<AgentEvent>,
    usage: &mut TokenUsage,
    used_bounded_sql_query: bool,
    tool_metadata: Vec<crate::ToolMetadata>,
) -> Result<AgentOutput, AgentError> {
    for call in pending {
        let (message, _) = tools::tool_message(
            call.id.clone(),
            serde_json::json!({"error": "tool call budget exhausted"}),
            context_byte_budget,
        );
        messages.push(message);
    }
    messages.push(crate::ChatMessage::text("user", SALVAGE_INSTRUCTION));
    let (assistant, turn_usage, _) =
        receive::receive(provider, model, messages, &[], sink, cancellation, events).await?;
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
    emit(events, sink, AgentEvent::Complete).await;
    Ok(AgentOutput {
        answer: assistant.content,
        events: std::mem::take(events),
        used_bounded_sql_query,
        tool_metadata,
        usage: *usage,
        learning_usage: None,
        truncated: true,
        answer_sql: None,
    })
}

/// The SQL the model designated as the answering query, when the terminal turn
/// contains a `designate_answer` call whose `sql` argument is a string. `None`
/// for a turn without the call (or a malformed argument) so the protocol stays
/// optional.
fn designation_from(assistant: &crate::ChatMessage) -> Option<String> {
    assistant.tool_calls.iter().find_map(|call| {
        if call.name == DESIGNATE_ANSWER_TOOL {
            call.arguments
                .get("sql")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        } else {
            None
        }
    })
}
