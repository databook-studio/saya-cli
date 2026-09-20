//! Push-guard arms: learning usage stays out of the session segment; zero-input/output turns still push; the footer stays a System block (moved verbatim from `streaming_tests.rs`, via
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

/// Runs one turn through `drain_stream` and returns the footer text it pushed.
/// The footer is the `tokens in` system block — not necessarily the last
/// block, since the context warning (when it fires) is pushed after it.
fn run_turn(app: &mut App, state: &mut SessionState, messages: Vec<StreamMsg>) -> String {
    app.request.stream = Some(stream_with(messages));
    assert!(app.drain_stream(state), "the turn finished");
    app.transcript
        .blocks()
        .iter()
        .rev()
        .find(|b| b.text.contains("tokens in"))
        .expect("the footer was pushed")
        .text
        .clone()
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

/// The extraction call's usage rides the stream as an `Extraction` report and
/// on `learning_usage`; neither may enter the footer's session segment, which
/// counts answering calls only. `/usage` keeps the detail.
#[test]
fn learning_usage_never_enters_the_footer_session_segment() {
    let (mut app, mut state) = app_with_model("mystery-model");
    app.request.stream = Some(stream_with(vec![
        StreamMsg::Event(AgentEvent::usage(
            UsageCall::Extraction,
            TokenUsage::new(900_000, 10),
        )),
        StreamMsg::Done(Ok(AgentOutput {
            learning_usage: Some(TokenUsage::new(900_000, 10)),
            ..output(TokenUsage::new(300, 100))
        })),
    ]));
    assert!(app.drain_stream(&mut state), "the turn finished");
    let footer = app
        .transcript
        .blocks()
        .last()
        .expect("the footer was pushed")
        .text
        .clone();
    assert_eq!(
        footer, "300 tokens in · 100 tokens out · session 300 in / 100 out",
        "the session segment is answering-only: {footer}"
    );
    assert_eq!(
        state.usage.learning.turns, 1,
        "the extraction call landed in the learning total for /usage"
    );
    assert_eq!(
        state.usage.answering.turns, 1,
        "the answering total counted exactly the answering call"
    );
}

/// The push guard's `||` arm: a provider that reported output but no input
/// still bought the user a footer, with the zero stated honestly. Dropping
/// this arm (or tightening the guard to `input > 0`) would hide the turn.
#[test]
fn a_turn_with_zero_input_but_reported_output_pushes_the_footer() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    // A fully silent turn first: no footer, and the session total untouched.
    app.request.stream = Some(stream_with(vec![StreamMsg::Done(Ok(output(
        TokenUsage::new(0, 0),
    )))]));
    assert!(app.drain_stream(&mut state), "the silent turn finished");
    assert_eq!(
        state.usage.answering.turns, 0,
        "a silent turn records nothing"
    );

    let footer = run_turn(
        &mut app,
        &mut state,
        vec![StreamMsg::Done(Ok(output(TokenUsage::new(0, 100))))],
    );
    assert_eq!(
        footer, "0 tokens in · 100 tokens out · session 0 in / 100 out",
        "an output-only turn pushes a footer with the honest zero input: {footer}"
    );
}

/// The other `||` arm: reported input with zero output. A guard of
/// `output > 0` alone would silently swallow input-only billing.
#[test]
fn a_turn_with_reported_input_but_zero_output_pushes_the_footer() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    let footer = run_turn(
        &mut app,
        &mut state,
        vec![StreamMsg::Done(Ok(output(TokenUsage::new(100, 0))))],
    );
    assert_eq!(
        footer, "100 tokens in · 0 tokens out · session 100 in / 0 out",
        "an input-only turn pushes a footer: {footer}"
    );
}

/// The footer block stays a System block. Copy, export, and session
/// persistence classify on kind; a reclassification would silently turn
/// plumbing into answer text.
#[test]
fn the_footer_block_is_a_system_block() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    app.request.stream = Some(stream_with(vec![
        answering_report(64_000),
        StreamMsg::Done(Ok(output(TokenUsage::new(64_000, 20)))),
    ]));
    assert!(app.drain_stream(&mut state), "the turn finished");
    let last = app
        .transcript
        .blocks()
        .last()
        .expect("the footer was pushed");
    assert_eq!(
        last.kind,
        BlockKind::System,
        "the footer must remain a System block"
    );
}
