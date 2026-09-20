//! Tests for the per-turn usage footer pushed by `drain_stream`: the
//! cumulative session totals and the context-window utilisation must be
//! visible without a slash command, and every unknown must stay absent.
//!
//! The turn is driven through the real seam — messages on the stream channel
//! drained by `drain_stream` — so the footer wording, the session totals, the
//! numerator capture, and the reset between turns are all exercised together.

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

/// Deliverable 2: the cumulative session total is visible without a slash
/// command. A second turn's footer must show the totals including every prior
/// turn, not just the turn that just finished.
#[test]
fn the_turn_footer_carries_session_totals_without_a_slash_command() {
    let (mut app, mut state) = app_with_model("mystery-model");
    // A prior turn's usage, folded the way `drain_stream` folds it.
    state.usage.record(&TokenUsage::new(1000, 500));

    let footer = run_turn(
        &mut app,
        &mut state,
        vec![StreamMsg::Done(Ok(output(TokenUsage::new(300, 100))))],
    );
    assert_eq!(
        footer, "300 tokens in · 100 tokens out · session 1300 in / 600 out",
        "the footer must carry the cumulative session totals inline"
    );
}

/// Deliverable 3: with the model's window known (from the built-in table) and
/// a provider report of the input, the footer shows the utilisation.
#[test]
fn context_utilisation_renders_when_the_window_is_known() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    let footer = run_turn(
        &mut app,
        &mut state,
        vec![
            answering_report(64_000),
            StreamMsg::Done(Ok(output(TokenUsage::new(64_000, 20)))),
        ],
    );
    assert!(
        footer.contains("· ctx 50% of 128k"),
        "ctx must show the last reported input against the known window: {footer}"
    );
}

/// A model the window table does not know renders no ctx figure — an assumed
/// window or a `0%` placeholder would tell the reader something false.
#[test]
fn context_utilisation_is_absent_when_the_window_is_unknown() {
    let (mut app, mut state) = app_with_model("mystery-model");
    let footer = run_turn(
        &mut app,
        &mut state,
        vec![
            answering_report(64_000),
            StreamMsg::Done(Ok(output(TokenUsage::new(64_000, 20)))),
        ],
    );
    assert!(
        !footer.contains("ctx"),
        "an unknown window means no ctx figure: {footer}"
    );
}

/// The run aggregate at `Done` sums every round's re-sent conversation, so it
/// is not a context size. Without a per-call report there is no numerator and
/// no ctx figure — even though the aggregate itself is non-zero.
#[test]
fn context_utilisation_is_absent_without_a_per_call_input_report() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    let footer = run_turn(
        &mut app,
        &mut state,
        vec![StreamMsg::Done(Ok(output(TokenUsage::new(64_000, 20))))],
    );
    assert!(
        !footer.contains("ctx"),
        "no per-call report means no ctx figure: {footer}"
    );
}

/// The extraction call's prompt is not the conversation: its input count must
/// never become the context numerator.
#[test]
fn the_extraction_call_is_not_the_context_numerator() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    let footer = run_turn(
        &mut app,
        &mut state,
        vec![
            StreamMsg::Event(AgentEvent::usage(
                UsageCall::Extraction,
                TokenUsage::new(900_000, 10),
            )),
            StreamMsg::Done(Ok(output(TokenUsage::new(64_000, 20)))),
        ],
    );
    assert!(
        !footer.contains("ctx"),
        "the extraction prompt is not the conversation: {footer}"
    );
}

/// The user-declared `[ai] context_window_tokens` wins over the built-in
/// table, exactly as the config resolution contract pins it — including for a
/// gateway-served model the table has never heard of.
#[test]
fn a_declared_window_wins_over_the_table() {
    let (mut app, mut state) = app_with_model("mystery-model");
    let mut runtime = unused_runtime();
    runtime.resolved.ai.context_window_tokens = Some(500_000);
    app.runtime = Arc::new(runtime);

    let footer = run_turn(
        &mut app,
        &mut state,
        vec![
            answering_report(64_000),
            StreamMsg::Done(Ok(output(TokenUsage::new(64_000, 20)))),
        ],
    );
    assert!(
        footer.contains("· ctx 13% of 500k"),
        "the declared window must win over the table: {footer}"
    );
}

/// The numerator is per-turn state: a turn the provider said nothing about
/// shows no stale ctx figure from the previous turn, while the session totals
/// carry forward.
#[test]
fn context_utilisation_resets_between_turns() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    let first = run_turn(
        &mut app,
        &mut state,
        vec![
            answering_report(64_000),
            StreamMsg::Done(Ok(output(TokenUsage::new(64_000, 20)))),
        ],
    );
    assert!(
        first.contains("ctx 50% of 128k"),
        "the first turn shows utilisation: {first}"
    );

    let second = run_turn(
        &mut app,
        &mut state,
        vec![StreamMsg::Done(Ok(output(TokenUsage::new(5000, 200))))],
    );
    assert!(
        !second.contains("ctx"),
        "a turn without a report shows no stale ctx figure: {second}"
    );
    assert!(
        second.contains("session 69000 in / 220 out"),
        "session totals carry forward across turns: {second}"
    );
}

/// The existing guard survives: a turn that reported nothing pushes no footer
/// at all — no invented session line, no invented ctx figure.
#[test]
fn a_turn_that_reported_no_usage_pushes_no_footer() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    let before = app.transcript.blocks().len();
    app.request.stream = Some(stream_with(vec![StreamMsg::Done(Ok(output(
        TokenUsage::new(0, 0),
    )))]));
    assert!(app.drain_stream(&mut state), "the turn finished");
    assert_eq!(
        app.transcript.blocks().len(),
        before,
        "a usage-less turn pushes no footer"
    );
}
