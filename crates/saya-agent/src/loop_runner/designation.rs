//! Reads the optional `designate_answer` call by which the model names the SQL
//! that answers the question at the terminal turn, and bounds the one
//! follow-up turn taken when that designation arrives without prose.

use super::{check_cancelled, emit, tools};
use crate::{
    AgentError, AgentEvent, AgentEventSink, AgentOutput, CancellationToken, ChatMessage,
    DESIGNATE_ANSWER_TOOL, TokenUsage, ToolMetadata,
};

/// How many follow-up turns an empty designation may spend recovering its
/// prose. A hard bound, not configurable: the recovery rescues an answer the
/// model forgot to write, and one ordinary turn — under every ceiling the run
/// already imposes — is enough.
const MAX_RECOVERIES: usize = 1;

/// The SQL the model designated as the answering query, when the terminal turn
/// contains a `designate_answer` call whose `sql` argument is a string. `None`
/// for a turn without the call (or a malformed argument) so the protocol stays
/// optional.
fn designation_from(assistant: &ChatMessage) -> Option<String> {
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

/// What the designation arm decided for the turn.
pub(super) enum Outcome {
    /// The run ended — a designation that carried its prose, or one whose
    /// recovery is spent.
    Done(Box<AgentOutput>),
    /// The designation carried no prose: the replies for its tool calls are
    /// recorded on the conversation and the loop continues into the one
    /// bounded follow-up turn.
    Recovering,
    /// No designation in this turn: the loop's own terminal handling applies.
    NotDesignated,
}

/// Where the designation path emits: the event list to append to, the sink to
/// stream through, and the cancellation token its checks consult.
type Emit<'a> = (
    &'a mut Vec<AgentEvent>,
    &'a dyn AgentEventSink,
    &'a CancellationToken,
);

/// The answer-side accumulators the terminal paths fold into [`AgentOutput`]:
/// the token counts (copied), whether a bounded SQL query ran, and the tool
/// metadata (taken on completion).
type Totals<'a> = (TokenUsage, bool, &'a mut Vec<ToolMetadata>);

/// Cross-turn state of the designation protocol within one run: the SQL of the
/// first designation (later ones are ignored, so a re-designation cannot move
/// the answer's query under an event already emitted), whether that event was
/// emitted, how many follow-up turns recovery has spent, and whether the next
/// turn owes the recovered prose.
pub(super) struct State {
    context_byte_budget: usize,
    pub(super) sql: Option<String>,
    emitted: bool,
    recoveries: usize,
    prose_due: bool,
}

impl State {
    pub(super) fn new(context_byte_budget: usize) -> Self {
        Self {
            context_byte_budget,
            sql: None,
            emitted: false,
            recoveries: 0,
            prose_due: false,
        }
    }

    /// Whether the turn in flight is the follow-up owed for an empty
    /// designation — the one turn whose failure degrades the run instead of
    /// failing it. Set when a recovery is taken; cleared when the turn's
    /// response arrives, so a continuation failure degrades with it and any
    /// later turn's failure is an ordinary failure. Cancellation is consulted
    /// through [`Emit`]'s token.
    pub(super) fn recovering(&self) -> bool {
        self.prose_due
    }
}

/// Handles a turn that may contain a `designate_answer` call. With prose, the
/// run ends as it always did. With empty prose — the failure real models hit,
/// sending the call with no sentence around it — the run does not end: the SQL
/// is recorded (first designation wins), the event is emitted once, every tool
/// call in the message is answered without executing (a provider rejects a
/// request whose assistant tool calls lack replies), and the loop takes one
/// ordinary follow-up turn — `max_turns`, `max_tool_calls` and cancellation
/// bind as for any turn — to produce the prose. Whatever later ends the run
/// carries the first designation.
pub(super) async fn handle(
    state: &mut State,
    assistant: &ChatMessage,
    (events, sink, cancellation): Emit<'_>,
    messages: &mut Vec<ChatMessage>,
    totals: Totals<'_>,
) -> Result<Outcome, AgentError> {
    state.prose_due = false;
    let Some(sql) = designation_from(assistant) else {
        return Ok(Outcome::NotDesignated);
    };
    if state.sql.is_none() {
        state.sql = Some(sql);
    }
    if !assistant.content.trim().is_empty() {
        emit_event(state, (events, sink, cancellation)).await;
        check_cancelled(cancellation)?;
        emit(events, sink, AgentEvent::Complete).await;
        return Ok(Outcome::Done(Box::new(completed_output(
            state, assistant, events, totals,
        ))));
    }
    if state.recoveries < MAX_RECOVERIES {
        state.recoveries += 1;
        state.prose_due = true;
        emit_event(state, (events, sink, cancellation)).await;
        answer_calls_without_executing(state, assistant, messages);
        return Ok(Outcome::Recovering);
    }
    // The recovery is spent and the follow-up still produced no prose:
    // complete with what the run has — the designated SQL, an empty answer —
    // rather than loop.
    check_cancelled(cancellation)?;
    emit(events, sink, AgentEvent::Complete).await;
    Ok(Outcome::Done(Box::new(completed_output(
        state, assistant, events, totals,
    ))))
}

/// The output when the follow-up turn failed before producing anything: the
/// designation succeeded, so the run ends with the SQL it already has and no
/// prose, rather than with an error. Cancellation is the user stopping the
/// run — it propagates, without a `Complete`.
pub(super) async fn failed_follow_up(
    state: &State,
    error: AgentError,
    (events, sink): (&mut Vec<AgentEvent>, &dyn AgentEventSink),
    totals: (TokenUsage, bool, Vec<ToolMetadata>),
) -> Result<AgentOutput, AgentError> {
    if matches!(
        error,
        AgentError::Cancelled | AgentError::Provider(crate::ProviderError::Cancelled)
    ) {
        return Err(error);
    }
    emit(events, sink, AgentEvent::Complete).await;
    let (usage, used_bounded_sql_query, tool_metadata) = totals;
    Ok(AgentOutput {
        answer: String::new(),
        events: std::mem::take(events),
        used_bounded_sql_query,
        tool_metadata,
        usage,
        learning_usage: None,
        truncated: false,
        answer_sql: state.sql.clone(),
    })
}

/// The output a designation ends the run with: the turn's prose as the answer
/// (empty when the recovery is spent without prose) and the first designation
/// as its SQL.
fn completed_output(
    state: &State,
    assistant: &ChatMessage,
    events: &mut Vec<AgentEvent>,
    totals: Totals<'_>,
) -> AgentOutput {
    let (usage, used_bounded_sql_query, tool_metadata) = totals;
    AgentOutput {
        answer: assistant.content.clone(),
        events: std::mem::take(events),
        used_bounded_sql_query,
        tool_metadata: std::mem::take(tool_metadata),
        usage,
        learning_usage: None,
        truncated: false,
        answer_sql: state.sql.clone(),
    }
}

/// Emits `AnswerDesignated` exactly once per run, on the first designation.
async fn emit_event(state: &mut State, (events, sink, _): Emit<'_>) {
    if !state.emitted
        && let Some(sql) = state.sql.clone()
    {
        emit(events, sink, AgentEvent::answer_designated(sql)).await;
        state.emitted = true;
    }
}

/// Builds the `tool` replies for every call of the empty-designation message,
/// none of which is executed: the designation call is told its SQL is recorded
/// and the prose is due; sibling calls are refused — executing them would
/// spend budget on work the run no longer needs. Each call gets a reply
/// because a provider rejects a request whose assistant tool calls lack them.
fn answer_calls_without_executing(
    state: &State,
    assistant: &ChatMessage,
    messages: &mut Vec<ChatMessage>,
) {
    let designated = state.sql.clone().unwrap_or_default();
    for call in &assistant.tool_calls {
        let result = if call.name == DESIGNATE_ANSWER_TOOL {
            recovery_result(&designated)
        } else {
            sibling_result()
        };
        let (message, _) = tools::tool_message(call.id.clone(), result, state.context_byte_budget);
        messages.push(message);
    }
}

/// The tool result answering the designation call in a recovery: the SQL is
/// recorded, and the model's one remaining job is to write the user-facing
/// prose. Worded to close the door on further tool calls — the run has its
/// answer, only the sentence around it is missing.
fn recovery_result(sql: &str) -> serde_json::Value {
    serde_json::json!({
        "recorded_sql": sql,
        "next": "The answering SQL is recorded. Now write the answer for the user in plain prose, without further tool calls."
    })
}

/// The tool result for a call riding the same message as an empty designation:
/// the run's answer already exists, so the call is answered without executing.
fn sibling_result() -> serde_json::Value {
    serde_json::json!({"error": "not executed: the answer was already designated"})
}
