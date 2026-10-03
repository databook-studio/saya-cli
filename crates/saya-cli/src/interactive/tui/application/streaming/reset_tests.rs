//! Reset and ordering invariants: TurnReset through the channel path and post-Done reports (moved verbatim from `streaming_tests.rs`, via
//! `streaming/footer_tests.rs`).

use super::*;

/// An `App` plus a session whose model is `model`. The runtime's declared
/// window is `None`, so the footer's window lookup must fall through to the
/// built-in table (which knows only its own models).
fn app_with_model(model: &str) -> (App, SessionState) {
    (idle_app(), SessionState::new("s1", None, model))
}

/// A stream that carries exactly `messages`, as the agent thread would.
fn stream_with(messages: Vec<StreamMsg>) -> Stream {
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

/// A run output carrying only the answering usage.
fn output(usage: TokenUsage) -> AgentOutput {
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

/// One answering round's usage report, as `receive` emits it mid-turn.
fn answering_report(input: u64) -> StreamMsg {
    StreamMsg::Event(AgentEvent::usage(
        UsageCall::Answer,
        TokenUsage::new(input, 20),
    ))
}

/// A reset mid-answer (`TurnReset`, from a mid-stream provider failure and a
/// turn retry) drives the whole channel path: the partial text drained before
/// the reset is discarded, and the re-streamed answer replaces it in the
/// transcript — never concatenated — and the spinner falls back to thinking.
#[test]
fn a_turn_reset_discards_the_partial_answer_and_the_retry_replaces_it() {
    let (mut app, mut state) = app_with_model("mystery-model");
    app.request.stream = Some(stream_with(vec![
        StreamMsg::Event(AgentEvent::assistant_text("The an")),
        StreamMsg::Event(AgentEvent::turn_reset()),
        StreamMsg::Event(AgentEvent::assistant_text("The answer is 42.")),
        StreamMsg::Done(Ok(output(TokenUsage::new(30, 20)))),
    ]));
    assert!(app.drain_stream(&mut state), "the turn finished");

    let assistant: Vec<_> = app
        .transcript
        .blocks()
        .iter()
        .filter(|b| b.kind == BlockKind::Assistant)
        .collect();
    assert_eq!(
        assistant.len(),
        1,
        "the retried answer stays one assistant block: {:?}",
        assistant
    );
    assert_eq!(
        assistant[0].text, "The answer is 42.",
        "the partial attempt's text must be discarded at the reset, not kept: {:?}",
        assistant
    );
    assert!(
        app.request.activity.is_none(),
        "after Done the request is finished; the reset also cleared activity mid-turn"
    );
}

/// A usage report processed after `Done` within one drained batch cannot
/// retroactively inject a ctx figure — and, the invariant that matters, it
/// must not survive the finished turn: the reset runs after the whole batch,
/// so the next turn starts with no numerator whatever the message order.
#[test]
fn a_report_drained_after_done_cannot_leak_into_the_next_turn() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    app.request.stream = Some(stream_with(vec![
        StreamMsg::Done(Ok(output(TokenUsage::new(64_000, 20)))),
        answering_report(64_000),
    ]));
    assert!(app.drain_stream(&mut state), "the turn finished");
    let footer = app
        .transcript
        .blocks()
        .last()
        .expect("the footer was pushed")
        .text
        .clone();
    assert!(
        !footer.contains("ctx"),
        "the footer is built from the state at Done-processing time: {footer}"
    );
    assert!(
        app.request.last_answering_input.is_none(),
        "the reset covers the whole batch, however it was ordered"
    );
}
