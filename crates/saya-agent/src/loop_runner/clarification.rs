//! The clarification arm (B3c): a `request_clarification` call ends the turn.
//! The question is surfaced as [`AgentEvent::ClarificationNeeded`], a short
//! tool result is fed for every call in the message, and no further provider
//! call happens until the user answers. Malformed asks are tool errors the
//! turn continues from — they never end a run, and never fabricate a question.
//! The argument contract ([`clarification_args`]) and the result feeding
//! ([`clarification_feed`]) are sibling concerns.

use super::clarification_args::{
    Clarification, ClarificationError, REQUEST_CLARIFICATION_TOOL, parse,
};
use super::clarification_feed::{
    NOT_EXECUTED_REFUSAL, PAUSED_REFUSAL, feed_malformed, gate_denial, refuse_others,
};
use super::{check_cancelled, designation, emit, output, tool_record, tools};
use crate::{
    AgentError, AgentEvent, AgentEventSink, AgentLimits, AgentOutput, ApprovalDecider,
    CancellationToken, ChatMessage, TokenUsage, ToolCall, ToolDefinition, ToolExecutor,
    ToolMetadata,
};

/// What the clarification arm decided for a turn carrying at least one
/// `request_clarification` call.
pub(super) enum Outcome {
    /// No such call in the message: the ordinary tool path applies untouched.
    NotCalled,
    /// No ask landed (every call malformed, or the executor refused it): the
    /// results are fed and the loop continues into the next turn.
    Continue,
    /// The ask landed: the question was surfaced and the turn ended.
    Done(Box<AgentOutput>),
}

/// Where the arm emits: the event list, the sink, and the cancellation token.
type Emit<'a> = (
    &'a mut Vec<AgentEvent>,
    &'a dyn AgentEventSink,
    &'a CancellationToken,
);

/// The answer-side accumulators, as [`designation`] takes them.
type Totals<'a> = (TokenUsage, bool, &'a mut Vec<ToolMetadata>);

/// Runs the clarification arm over one assistant message. Called before the
/// ordinary tool path; when it returns [`Outcome::Done`] the run is over.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle(
    assistant: &ChatMessage,
    definitions: &[ToolDefinition],
    limits: &AgentLimits,
    approval: &dyn ApprovalDecider,
    executor: &dyn ToolExecutor,
    designation: &designation::State,
    (events, sink, cancellation): Emit<'_>,
    messages: &mut Vec<ChatMessage>,
    totals: Totals<'_>,
) -> Result<Outcome, AgentError> {
    // A call with no id cannot anchor a tool result; leave the message to the
    // ordinary path, which fails closed on exactly that.
    if assistant
        .tool_calls
        .iter()
        .any(|call| call.id.trim().is_empty())
    {
        return Ok(Outcome::NotCalled);
    }
    let parsed: Vec<(&ToolCall, Result<Clarification, ClarificationError>)> = assistant
        .tool_calls
        .iter()
        .filter(|call| call.name == REQUEST_CLARIFICATION_TOOL)
        .map(|call| (call, parse(&call.arguments)))
        .collect();
    if parsed.is_empty() {
        return Ok(Outcome::NotCalled);
    }
    let Some((asked_index, asked)) = parsed
        .iter()
        .enumerate()
        .find_map(|(index, (_, result))| result.as_ref().ok().map(|ask| (index, ask.clone())))
    else {
        // Every ask in the message is malformed: each is fed its own named
        // validation error and the siblings are refused, so the model can
        // retry — the turn continues, and nothing asks.
        check_cancelled(cancellation)?;
        for (call, result) in &parsed {
            feed_malformed(
                call,
                result.as_ref().expect_err("branch holds only errors"),
                definitions,
                limits,
                (events, sink),
                messages,
                totals.2,
            )
            .await;
        }
        refuse_others(assistant, &[], limits, messages, NOT_EXECUTED_REFUSAL);
        return Ok(Outcome::Continue);
    };
    let asked_call = parsed[asked_index].0;
    check_cancelled(cancellation)?;
    let definition = definitions
        .iter()
        .find(|definition| definition.name == REQUEST_CLARIFICATION_TOOL);
    emit(
        events,
        sink,
        AgentEvent::tool_requested(
            asked_call.name.clone(),
            asked_call.arguments.clone(),
            definition.map(|definition| definition.effect),
        ),
    )
    .await;
    // The gates the sequential path applies to any call, consulted for this
    // one: a mis-declared definition is denied rather than trusted. The
    // production definition declares no effect and no approval need, so it
    // always passes; the arm never widens what the declaration admits.
    let denial = gate_denial(definition, limits, approval, &asked_call.arguments).await;
    let (result, summary) = if let Some(reason) = denial {
        emit(
            events,
            sink,
            AgentEvent::ToolDenied {
                name: REQUEST_CLARIFICATION_TOOL.into(),
                reason: reason.clone(),
            },
        )
        .await;
        (
            serde_json::json!({"error": format!("tool call denied by approval policy: {reason}")}),
            "read-only database tool denied".to_owned(),
        )
    } else {
        tools::execute(
            executor,
            REQUEST_CLARIFICATION_TOOL,
            asked_call.arguments.clone(),
            definition,
        )
        .await
    };
    let failed = summary.contains("failed");
    totals.2.push(ToolMetadata {
        name: REQUEST_CLARIFICATION_TOOL.into(),
        status: if failed { "failed" } else { "completed" }.into(),
        arguments: serde_json::to_string(&asked_call.arguments).unwrap_or_default(),
        result_shape: tool_record::result_shape_of(&result),
    });
    let (message, truncated) =
        tools::tool_message(asked_call.id.clone(), result, limits.context_byte_budget);
    messages.push(message);
    emit(
        events,
        sink,
        AgentEvent::ToolCompleted {
            name: REQUEST_CLARIFICATION_TOOL.into(),
            summary: output::completion_summary(&summary, truncated),
        },
    )
    .await;
    if failed {
        refuse_others(
            assistant,
            &[asked_call.id.as_str()],
            limits,
            messages,
            NOT_EXECUTED_REFUSAL,
        );
        return Ok(Outcome::Continue);
    }
    emit(
        events,
        sink,
        AgentEvent::clarification_needed(asked.question, asked.options),
    )
    .await;
    refuse_others(
        assistant,
        &[asked_call.id.as_str()],
        limits,
        messages,
        PAUSED_REFUSAL,
    );
    check_cancelled(cancellation)?;
    emit(events, sink, AgentEvent::Complete).await;
    let (usage, used_bounded_sql_query, tool_metadata) = totals;
    Ok(Outcome::Done(Box::new(AgentOutput {
        answer: assistant.content.clone(),
        events: std::mem::take(events),
        used_bounded_sql_query,
        tool_metadata: std::mem::take(tool_metadata),
        usage,
        learning_usage: None,
        truncated: false,
        answer_sql: designation.sql.clone(),
    })))
}

#[cfg(test)]
#[path = "clarification_tests.rs"]
mod tests;
