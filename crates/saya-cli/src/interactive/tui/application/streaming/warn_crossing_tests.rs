//! Warn-threshold crossing: one notice on the upward crossing, silence while above, re-arm below (moved verbatim from `streaming_tests.rs`, via
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

/// Crossing the warn threshold upward emits exactly one notice; staying above
/// it for three more turns emits none. The notice teaches the behaviour
/// before it happens: percentage, window, and what fires at 95%.
#[test]
fn crossing_the_warn_threshold_emits_one_notice_then_stays_silent() {
    use saya_agent::CONTEXT_WARN_PERCENT;
    let (mut app, mut state) = app_with_model("gpt-4o");
    // 89_600 / 128_000 = exactly 70%.
    let warned = (CONTEXT_WARN_PERCENT * 128_000) / 100;
    let first = run_turn(
        &mut app,
        &mut state,
        vec![
            answering_report(warned),
            StreamMsg::Done(Ok(output(TokenUsage::new(warned, 20)))),
        ],
    );
    assert!(
        first.contains("· ctx 70% of 128k"),
        "the footer's existing output is unchanged: {first}"
    );
    let notices = notice_texts(&app);
    assert_eq!(notices.len(), 1, "one notice on the crossing: {notices:?}");
    let notice = &notices[0];
    assert!(
        notice.contains("70%"),
        "the notice states the percentage: {notice}"
    );
    assert!(
        notice.contains("128k"),
        "the notice states the window: {notice}"
    );
    assert!(
        notice.contains(&saya_agent::CONTEXT_COMPACT_PERCENT.to_string()),
        "the notice names what happens at the compact threshold: {notice}"
    );

    for turn in 1..=3 {
        let footer = run_turn(
            &mut app,
            &mut state,
            vec![
                answering_report(warned + 1_000),
                StreamMsg::Done(Ok(output(TokenUsage::new(warned + 1_000, 20)))),
            ],
        );
        assert!(
            footer.contains("ctx"),
            "staying above keeps the footer figure: {footer}"
        );
        assert_eq!(
            notice_texts(&app).len(),
            1,
            "turn {turn} above the threshold emits no second notice"
        );
    }
}

/// Falling below the threshold re-arms the warning; crossing again emits a
/// second notice. A turn with no provider report of the input leaves the flag
/// untouched — absence is not zero, so it neither fires nor re-arms.
#[test]
fn falling_below_re_arms_and_crossing_again_emits_a_second_notice() {
    use saya_agent::CONTEXT_WARN_PERCENT;
    let (mut app, mut state) = app_with_model("gpt-4o");
    let warned = (CONTEXT_WARN_PERCENT * 128_000) / 100;
    let _ = run_turn(
        &mut app,
        &mut state,
        vec![
            answering_report(warned),
            StreamMsg::Done(Ok(output(TokenUsage::new(warned, 20)))),
        ],
    );
    assert_eq!(notice_texts(&app).len(), 1);

    let below = run_turn(
        &mut app,
        &mut state,
        vec![
            answering_report(10_000),
            StreamMsg::Done(Ok(output(TokenUsage::new(10_000, 20)))),
        ],
    );
    assert!(
        below.contains("ctx 8% of 128k"),
        "below the threshold the footer still renders: {below}"
    );
    assert_eq!(
        notice_texts(&app).len(),
        1,
        "falling below re-arms silently, with no new notice"
    );

    // A silent turn (no per-call report) neither fires nor re-arms: after it,
    // crossing again still emits exactly the second notice.
    let silent = run_turn(
        &mut app,
        &mut state,
        vec![StreamMsg::Done(Ok(output(TokenUsage::new(10_000, 20))))],
    );
    assert!(
        !silent.contains("ctx"),
        "no per-call report means no ctx figure: {silent}"
    );
    assert_eq!(
        notice_texts(&app).len(),
        1,
        "a silent turn leaves the armed flag untouched"
    );

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
        2,
        "the second crossing emits a second notice"
    );
}
