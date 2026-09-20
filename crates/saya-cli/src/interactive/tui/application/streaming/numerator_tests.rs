//! Numerator orderings: the last answering report wins in multi-round turns; errored turns leave no stale numerator (moved verbatim from `streaming_tests.rs`, via
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

/// A two-round tool loop re-sends the conversation, so the aggregate at `Done`
/// sums both rounds — but the context numerator is the LAST answering report.
/// If the capture kept the first report or fell back to the aggregate, this
/// footer would misstate the context share on exactly the tool-heavy turns
/// where the figure matters.
#[test]
fn the_last_answering_report_wins_in_a_multi_round_turn() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    let footer = run_turn(
        &mut app,
        &mut state,
        vec![
            answering_report(10_000),
            answering_report(64_000),
            StreamMsg::Done(Ok(output(TokenUsage::new(74_000, 40)))),
        ],
    );
    assert!(
        footer.contains("74000 tokens in · 40 tokens out"),
        "the turn segment shows the run aggregate: {footer}"
    );
    assert!(
        footer.contains("· ctx 50% of 128k"),
        "the last answering round is the numerator, not the first or the sum: {footer}"
    );
}

/// The mirror ordering: the smaller report arrives last. `10_000 / 128_000`
/// rounds to 8% — a first-wins capture would render 50% instead.
#[test]
fn a_second_answering_report_replaces_the_first() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    let footer = run_turn(
        &mut app,
        &mut state,
        vec![
            answering_report(64_000),
            answering_report(10_000),
            StreamMsg::Done(Ok(output(TokenUsage::new(74_000, 40)))),
        ],
    );
    assert!(
        footer.contains("· ctx 8% of 128k"),
        "the most recent answering report must win: {footer}"
    );
}

/// An errored turn that had already reported usage: no footer, no session
/// total, and — the part a later turn would expose — no leftover numerator.
/// The reset lives in the shared finish block; if it moved into the Ok arm
/// only, the next turn would resurrect the errored turn's ctx figure.
#[test]
fn an_errored_turn_pushes_no_footer_and_leaves_no_stale_numerator() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    app.request.stream = Some(stream_with(vec![
        answering_report(64_000),
        StreamMsg::Done(Err("provider exploded".into())),
    ]));
    assert!(app.drain_stream(&mut state), "the errored turn finished");
    let last = app
        .transcript
        .blocks()
        .last()
        .expect("the error was pushed");
    assert_eq!(last.kind, BlockKind::Error, "the error block was pushed");
    assert!(
        !app.transcript
            .blocks()
            .iter()
            .any(|b| b.text.contains("tokens in")),
        "an errored turn pushes no usage footer"
    );
    assert!(
        app.request.last_answering_input.is_none(),
        "the errored turn's numerator is cleared"
    );
    assert_eq!(
        state.usage.answering.turns, 0,
        "an errored turn records no session usage"
    );

    // A later turn whose provider stays silent must show no stale ctx figure,
    // and the session totals must not include the errored turn.
    let footer = run_turn(
        &mut app,
        &mut state,
        vec![StreamMsg::Done(Ok(output(TokenUsage::new(64_000, 20))))],
    );
    assert!(
        !footer.contains("ctx"),
        "the errored turn's report must not leak into the next footer: {footer}"
    );
    assert!(
        footer.contains("session 64000 in / 20 out"),
        "the errored turn contributed nothing to the session totals: {footer}"
    );
}
