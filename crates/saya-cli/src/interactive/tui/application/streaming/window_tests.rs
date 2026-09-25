//! Window and model lookups behind the footer: live-model switches, unusual magnitudes, exact-match table behaviour (moved verbatim from `streaming_tests.rs`, via
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

/// `/model` mutates the live `state.model` mid-session via `SessionState::
/// apply` — the footer's table lookup must follow it. A window pinned to the
/// session's first model would misreport every turn after the switch.
#[test]
fn the_window_lookup_follows_the_live_model() {
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
        "the first turn resolves gpt-4o's window: {first}"
    );

    // Exactly what `/model claude-haiku-4-5` does mid-session.
    let _ = state.apply(
        crate::SlashCommand::Model(Some("claude-haiku-4-5".into())),
        &[],
    );

    let second = run_turn(
        &mut app,
        &mut state,
        vec![
            answering_report(64_000),
            StreamMsg::Done(Ok(output(TokenUsage::new(64_000, 20)))),
        ],
    );
    assert!(
        second.contains("· ctx 32% of 200k"),
        "the window must follow the live model, not the session's first one: {second}"
    );
}

/// The declared window passes config validation at any magnitude above zero.
/// A huge one must still render compactly without panicking, and the honest
/// rounding of a vanishing share is 0% — the window did not become unknown.
#[test]
fn a_declared_window_of_unusual_magnitude_still_renders() {
    let (mut app, mut state) = app_with_model("mystery-model");
    let mut runtime = unused_runtime();
    runtime.resolved.ai.context_window_tokens = Some(1_000_000_000_000);
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
        footer.ends_with("· ctx 0% of 1000000M"),
        "a trillion-token window renders compactly, honestly rounded: {footer}"
    );
}

/// A declared window of one token (valid per config) and a provider-reported
/// `u64::MAX` input must not panic: the percentage saturates the way Rust's
/// float-to-int cast saturates, and the window still renders beside it.
#[test]
fn a_one_token_window_with_a_huge_input_renders_without_panicking() {
    let (mut app, mut state) = app_with_model("mystery-model");
    let mut runtime = unused_runtime();
    runtime.resolved.ai.context_window_tokens = Some(1);
    app.runtime = Arc::new(runtime);

    let footer = run_turn(
        &mut app,
        &mut state,
        vec![
            answering_report(u64::MAX),
            StreamMsg::Done(Ok(output(TokenUsage::new(u64::MAX, 0)))),
        ],
    );
    assert!(
        footer.contains("ctx") && footer.ends_with("% of 1"),
        "a huge input over a one-token window renders (saturated), never panics: {footer}"
    );
}

/// A reported input larger than the window renders above 100% — the honest
/// reading of a turn that overflowed the window. Clamping the figure to 100%
/// would hide the very condition the reader needs to see.
#[test]
fn an_input_larger_than_the_window_renders_above_one_hundred_percent() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    let footer = run_turn(
        &mut app,
        &mut state,
        vec![
            answering_report(200_000),
            StreamMsg::Done(Ok(output(TokenUsage::new(200_000, 20)))),
        ],
    );
    assert!(
        footer.contains("· ctx 156% of 128k"),
        "utilisation above the window is rendered, not clamped: {footer}"
    );
}

/// The table lookup stays exact-match through the footer: `GPT-4O` is a model
/// the table does not know, and a case-folding shortcut here would contradict
/// the config contract that a case difference is enough to be unknown.
#[test]
fn the_table_lookup_stays_exact_match_through_the_footer() {
    let (mut app, mut state) = app_with_model("GPT-4O");
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
        "a case difference is enough to be unknown, even in the footer: {footer}"
    );
}
