use super::super::super::super::agent::{Stream, StreamMsg};
use super::super::super::super::types::App;
use super::super::super::tests_support::idle_app;
use crate::interactive::session_state::SessionState;
use saya_agent::{AgentEvent, AgentOutput, CancellationToken, TokenUsage, UsageCall};
use tokio::sync::mpsc::unbounded_channel;
pub(crate) fn app_with_model(model: &str) -> (App, SessionState) {
    (idle_app(), SessionState::new("s1", None, model))
}

pub(crate) fn stream_with(messages: Vec<StreamMsg>) -> Stream {
    let (tx, rx) = unbounded_channel();
    for message in messages {
        let _ = tx.send(message);
    }
    Stream {
        rx,
        cancel: CancellationToken::new(),
        prompt: "question".into(),
    }
}

pub(crate) fn output(usage: TokenUsage) -> AgentOutput {
    AgentOutput {
        answer: "the answer".into(),
        events: Vec::new(),
        used_bounded_sql_query: false,
        tool_metadata: Vec::new(),
        usage,
        learning_usage: None,
        truncated: false,
        answer_sql: None,
    }
}

pub(crate) fn answering_report(input: u64) -> StreamMsg {
    StreamMsg::Event(AgentEvent::usage(
        UsageCall::Answer,
        TokenUsage::new(input, 20),
    ))
}

/// Runs one turn through `drain_stream`, returning whether a compaction
/// worker started. The turn needs enough history for a plan to exist — five
/// recorded turns — so a firing decision actually starts the worker.
pub(crate) fn run_turn_maybe_compacting(
    app: &mut App,
    state: &mut SessionState,
    messages: Vec<StreamMsg>,
) -> bool {
    app.request.stream = Some(stream_with(messages));
    assert!(app.drain_stream(state), "the turn finished");
    app.compact_task.is_some()
}

pub(crate) fn five_turns() -> SessionState {
    let mut state = SessionState::new("s1", None, "gpt-4o");
    for index in 0..5 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    state
}
