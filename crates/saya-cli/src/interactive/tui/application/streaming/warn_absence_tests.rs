//! Warn-threshold absence: unknown windows and silent providers never warn and leave the flag untouched (moved verbatim from `streaming_tests.rs`, via
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

/// Every context notice the transcript carries (the footer's own `· ctx …`
/// segment excluded): the system blocks the warning path pushed.
fn notice_texts(app: &App) -> Vec<String> {
    app.transcript
        .blocks()
        .iter()
        .filter(|b| b.kind == BlockKind::System && !b.text.contains("tokens in"))
        .map(|b| b.text.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Adversarial additions: boundaries and orderings the tests above do not
// reach. Each pins a behaviour whose silent regression would mislead the
// reader of the footer rather than fail loudly.
// ---------------------------------------------------------------------------

/// An unknown window emits no notice at any usage level. The input here
/// (126_720) would be 99% under gpt-4o's window, so silence proves the
/// missing denominator — not the number — is what suppresses the warning.
#[test]
fn an_unknown_window_emits_no_notice_at_any_usage_level() {
    let (mut app, mut state) = app_with_model("mystery-model");
    let footer = run_turn(
        &mut app,
        &mut state,
        vec![
            answering_report(126_720),
            StreamMsg::Done(Ok(output(TokenUsage::new(126_720, 20)))),
        ],
    );
    assert!(
        !footer.contains("ctx"),
        "no window means no ctx figure: {footer}"
    );
    assert!(
        notice_texts(&app).is_empty(),
        "no window means no warning, even at 99%-of-a-known-window usage"
    );
}

/// A provider reporting no usage (all-zero, the silent-provider encoding)
/// emits no notice and does not fire the flag: absence means unknown, never
/// zero. A later crossing still warns exactly once.
#[test]
fn a_provider_reporting_no_usage_emits_no_notice_and_does_not_fire() {
    let (mut app, mut state) = app_with_model("gpt-4o");
    let before = app.transcript.blocks().len();
    app.request.stream = Some(stream_with(vec![StreamMsg::Done(Ok(output(
        TokenUsage::new(0, 0),
    )))]));
    assert!(app.drain_stream(&mut state), "the silent turn finished");
    assert_eq!(
        app.transcript.blocks().len(),
        before,
        "a usage-less turn pushes no footer"
    );
    assert!(
        notice_texts(&app).is_empty(),
        "absence of usage is not a zero to warn about"
    );

    use saya_agent::CONTEXT_WARN_PERCENT;
    let warned = (CONTEXT_WARN_PERCENT * 128_000) / 100;
    let _ = run_turn(
        &mut app,
        &mut state,
        vec![
            answering_report(warned),
            StreamMsg::Done(Ok(output(TokenUsage::new(warned, 20)))),
        ],
    );
    assert_eq!(
        notice_texts(&app).len(),
        1,
        "the silent turn left the flag unfired, so the crossing warns"
    );
}
