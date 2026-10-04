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
mod salvage;
mod tool_batch;
mod tool_policy;
mod tool_protocol;
mod tool_record;
mod tool_shape;
mod tools;
mod turn_tools;

pub use clarification_args::{
    MAX_CLARIFICATION_OPTION_CHARS, MAX_CLARIFICATION_OPTIONS, MAX_CLARIFICATION_QUESTION_CHARS,
    REQUEST_CLARIFICATION_TOOL,
};

use crate::{
    AgentEvent, AgentEventSink, AgentRequest, ApprovalDecider, CancellationToken, ChatMessage,
    ChatProvider, ProviderError, TokenUsage, ToolDefinition, ToolExecutor,
};

use control::AgentRunInvocation;
pub use control::{AgentRunControl, AnsweringLimits, AnsweringScope};
pub use output::{
    AgentError, AgentLimits, AgentOutput, DESIGNATE_ANSWER_TOOL, EnvBudgets, budgets_from_env,
};
pub use preflight::RequestPreflightConfig;
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
    run_agent_inner(AgentRunInvocation {
        provider,
        tools,
        request,
        definitions,
        limits,
        approval,
        sink,
        cancellation,
        answering: None,
        request_preflight: None,
    })
    .await
}

pub async fn run_agent_with_control<'a>(
    provider: &'a dyn ChatProvider,
    tools: &'a dyn ToolExecutor,
    request: AgentRequest,
    definitions: Vec<ToolDefinition>,
    control: AgentRunControl<'a>,
) -> Result<AgentOutput, AgentError> {
    run_agent_inner(AgentRunInvocation {
        provider,
        tools,
        request,
        definitions,
        limits: control.limits,
        approval: control.approval,
        sink: control.sink,
        cancellation: control.cancellation,
        answering: Some(control.answering_scope),
        request_preflight: control.request_preflight,
    })
    .await
}

async fn run_agent_inner(invocation: AgentRunInvocation<'_>) -> Result<AgentOutput, AgentError> {
    if let Some(scope) = invocation.answering.clone() {
        let mut admission = scope.lock().await?;
        admission.begin_invocation();
        run_agent_loop(invocation, &mut admission).await
    } else {
        let mut admission = admission::Admission::new(AnsweringLimits::default(), false);
        run_agent_loop(invocation, &mut admission).await
    }
}

async fn run_agent_loop(
    invocation: AgentRunInvocation<'_>,
    admission: &mut admission::Admission,
) -> Result<AgentOutput, AgentError> {
    let AgentRunInvocation {
        provider,
        tools,
        request,
        definitions,
        limits,
        approval,
        sink,
        cancellation,
        answering,
        request_preflight,
    } = invocation;
    let controlled = answering.is_some();
    admission.initialize(sink).await;
    if controlled {
        admission.check(&cancellation)?;
    }
    let (mut messages, mut ordinary_history_pairs, mut protected_prefix_len) =
        crate::history::build_messages_with_history_count(
            request.system_prompt.as_deref(),
            &request.context_blocks,
            &request.prompt,
            &request.history,
            limits.context_byte_budget,
        )?;
    let mut events = Vec::new();
    let mut used_bounded_sql_query = false;
    let mut tool_metadata = Vec::new();
    let mut usage = TokenUsage::default();
    let mut continuation_count = 0;
    let mut pending_continuation = None;
    // Calls that failed during this run — SQL statements keyed on the
    // statement exactly as submitted, other tools on (tool, arguments) — so a
    // byte-identical re-submission is refused rather than re-executed (loop
    // invariant for the "do not repeat a failed call" advice the model does
    // not always obey).
    let mut failed = failed_statements::FailedStatements::new();
    // The last statement that completed successfully, so a run that exhausts its
    // budget without nominating can still surface its best available answer.
    let mut last_successful_sql: Option<String> = None;
    let mut designation = designation::State::new();
    loop {
        if let Err(error) = admission.check(&cancellation) {
            if let Some(dimension) = admission.stopped_dimension(&error, &cancellation) {
                return Ok(budget_output(
                    dimension,
                    &mut events,
                    sink,
                    PartialOutput {
                        usage,
                        used_bounded_sql_query,
                        tool_metadata,
                        answer_sql: designation.sql.clone().or(last_successful_sql),
                        answer: String::new(),
                    },
                )
                .await);
            }
            return Err(error);
        }
        if admission.legacy.turn_limit(limits.max_turns) {
            return salvage::salvage(
                provider,
                &request.model,
                &mut messages,
                &mut ordinary_history_pairs,
                &mut protected_prefix_len,
                &[],
                limits.context_byte_budget,
                request_preflight,
                sink,
                &cancellation,
                &mut events,
                &mut usage,
                admission,
                used_bounded_sql_query,
                tool_metadata,
                designation.sql.clone().or(last_successful_sql),
            )
            .await;
        }
        admission.legacy.admit_turn();
        // A truncation is deterministic — re-sending the identical request fails
        // identically, so `receive` never retries it. Within budget the loop
        // instead re-instructs: the partial stays discarded, one user-role note
        // tells the model to resume compactly, and the turn is spent — the
        // The shared admission owner has already charged this turn, so `max_turns` keeps binding.
        let (assistant, turn_usage, reasoning) = match receive::receive(receive::ReceiveContext {
            provider,
            model: &request.model,
            messages: &mut messages,
            ordinary_history_pairs: &mut ordinary_history_pairs,
            protected_prefix_len: &mut protected_prefix_len,
            definitions: &definitions,
            context_byte_budget: limits.context_byte_budget,
            request_preflight,
            continuation: pending_continuation.take(),
            sink,
            cancellation: &cancellation,
            events: &mut events,
            admission,
        })
        .await
        {
            Ok(turn) => turn,
            Err(failure) => {
                add_usage(&mut usage, failure.usage);
                let error = if matches!(&failure.error, AgentError::Cancelled) {
                    admission
                        .check(&cancellation)
                        .err()
                        .unwrap_or(failure.error)
                } else {
                    failure.error
                };
                if cancellation.reason() == Some(crate::CancellationReason::OwnerFailure) {
                    return Err(error);
                }
                match error {
                    AgentError::Provider(ProviderError::OutputTruncated { .. })
                        if limits
                            .max_continuations
                            .is_some_and(|max| continuation_count < max) =>
                    {
                        continuation_count += 1;
                        pending_continuation = Some(receive::ContinuationProgress {
                            index: continuation_count as usize,
                            limit: limits
                                .max_continuations
                                .expect("the continuation guard checked a finite limit")
                                as usize,
                        });
                        messages.push(ChatMessage::text("user", CONTINUATION_NOTE));
                        continue;
                    }
                    error if admission.stopped_dimension(&error, &cancellation).is_some() => {
                        let dimension = admission
                            .stopped_dimension(&error, &cancellation)
                            .expect("guarded budget reason");
                        return Ok(budget_output(
                            dimension,
                            &mut events,
                            sink,
                            PartialOutput {
                                usage,
                                used_bounded_sql_query,
                                tool_metadata,
                                answer_sql: designation.sql.clone().or(last_successful_sql),
                                answer: String::new(),
                            },
                        )
                        .await);
                    }
                    error if designation.recovering() => {
                        return designation::failed_follow_up(
                            &designation,
                            error,
                            (&mut events, sink),
                            (usage, used_bounded_sql_query, tool_metadata),
                        )
                        .await;
                    }
                    error => return Err(error),
                }
            }
        };
        // Providers report cumulative counts per response; sum across turns.
        add_usage(&mut usage, turn_usage);
        if !assistant.tool_calls.is_empty()
            && let Err(error) = admission.enforce_tools(&cancellation)
            && let Some(dimension) = admission.stopped_dimension(&error, &cancellation)
        {
            return Ok(budget_output(
                dimension,
                &mut events,
                sink,
                PartialOutput {
                    usage,
                    used_bounded_sql_query,
                    tool_metadata,
                    answer_sql: designation.sql.clone().or(last_successful_sql),
                    answer: assistant.content,
                },
            )
            .await);
        }
        let mut response_limits = limits;
        let result_caps = if !assistant.tool_calls.is_empty() {
            let result_budget = match preflight::reserve_tool_batch(
                &request.model,
                &mut messages,
                protected_prefix_len,
                &assistant,
                &definitions,
                limits.context_byte_budget,
                request_preflight,
            ) {
                Ok(budget) => budget,
                Err(preflight::PreflightError::Budget(dimension)) => {
                    let error = admission.stop_preflight(&cancellation, dimension);
                    if let Some(dimension) = admission.stopped_dimension(&error, &cancellation) {
                        messages.push(assistant.clone());
                        messages.extend(preflight::non_execution_replies(&assistant));
                        return Ok(budget_output(
                            dimension,
                            &mut events,
                            sink,
                            PartialOutput {
                                usage,
                                used_bounded_sql_query,
                                tool_metadata,
                                answer_sql: designation.sql.clone().or(last_successful_sql),
                                answer: assistant.content,
                            },
                        )
                        .await);
                    }
                    return Err(error);
                }
                Err(preflight::PreflightError::Serialization) => {
                    return Err(AgentError::Limit("request projection serialization failed"));
                }
            };
            response_limits.context_byte_budget =
                response_limits.context_byte_budget.min(result_budget);
            tool_batch::turn_result_caps(result_budget, assistant.tool_calls.len())
        } else {
            Vec::new()
        };
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
        // Designation arm: see `designation` for the bounded prose recovery.
        match designation::handle(
            &mut designation,
            &assistant,
            &result_caps,
            (&mut events, sink, &cancellation),
            &mut messages,
            (usage, used_bounded_sql_query, &mut tool_metadata),
        )
        .await?
        {
            designation::Outcome::Done(output) => return Ok(*output),
            designation::Outcome::Recovering => continue,
            designation::Outcome::NotDesignated => {}
        }
        // The clarification arm (B3c): a `request_clarification` call ends the
        // turn — the question is surfaced, the tool result is fed, and no
        // further provider call happens until the user answers. A malformed
        // ask is fed back as a tool error and the turn continues.
        match clarification::handle(
            &assistant,
            &definitions,
            &response_limits,
            &result_caps,
            approval,
            tools,
            &designation,
            (&mut events, sink, &cancellation),
            &mut messages,
            (usage, used_bounded_sql_query, &mut tool_metadata),
        )
        .await?
        {
            clarification::Outcome::Done(output) => return Ok(*output),
            // The arm answered this message's calls (errors for malformed
            // asks, refusals for the siblings): its results are already on
            // the conversation, and the next turn retries — the ordinary
            // path must not run the same calls again.
            clarification::Outcome::Continue => continue,
            clarification::Outcome::NotCalled => {}
        }
        if assistant.tool_calls.is_empty() {
            if !matches!(
                cancellation.reason(),
                Some(crate::CancellationReason::Budget(_))
            ) {
                check_cancelled(&cancellation)?;
            }
            emit(&mut events, sink, AgentEvent::Complete).await;
            return Ok(AgentOutput {
                answer: assistant.content,
                events,
                used_bounded_sql_query,
                tool_metadata,
                usage,
                learning_usage: None,
                truncated: false,
                answer_sql: designation.sql.clone(),
            });
        }
        // The tool-call ceiling is a whole-run total checked once per turn,
        // before any of the turn's calls run: if this turn would cross it, the
        // run stops rather than executing a partial batch.
        if admission
            .legacy
            .tool_limit(limits.max_tool_calls, assistant.tool_calls.len())
        {
            return salvage::salvage(
                provider,
                &request.model,
                &mut messages,
                &mut ordinary_history_pairs,
                &mut protected_prefix_len,
                &assistant.tool_calls,
                limits.context_byte_budget,
                request_preflight,
                sink,
                &cancellation,
                &mut events,
                &mut usage,
                admission,
                used_bounded_sql_query,
                tool_metadata,
                designation.sql.clone().or(last_successful_sql),
            )
            .await;
        }
        admission.legacy.admit_tools(assistant.tool_calls.len());
        let partial_answer = assistant.content.clone();
        let tool_result = turn_tools::run_turn_tools(
            tools,
            assistant,
            &definitions,
            &result_caps,
            &response_limits,
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
        .await;
        if let Err(error) = tool_result {
            let error = if matches!(&error, AgentError::Cancelled) {
                admission.check(&cancellation).err().unwrap_or(error)
            } else {
                error
            };
            if let Some(dimension) = admission.stopped_dimension(&error, &cancellation) {
                return Ok(budget_output(
                    dimension,
                    &mut events,
                    sink,
                    PartialOutput {
                        usage,
                        used_bounded_sql_query,
                        tool_metadata,
                        answer_sql: designation.sql.clone().or(last_successful_sql),
                        answer: partial_answer,
                    },
                )
                .await);
            }
            return Err(error);
        }
        // Intra-loop context budget: the pre-loop trim bounds history, but
        // assistant turns and tool results accumulate here. Trim every turn —
        // batch (parallel) and sequential alike — so the byte budget binds
        // unconditionally, never just on the turns a single call ran.
        output::trim_to_budget(
            &mut messages,
            limits.context_byte_budget,
            protected_prefix_len,
        );
    }
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
fn check_cancelled(token: &CancellationToken) -> Result<(), AgentError> {
    if token.is_cancelled() {
        Err(AgentError::Cancelled)
    } else {
        Ok(())
    }
}
