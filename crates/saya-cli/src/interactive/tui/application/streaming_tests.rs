//! Tests for the per-turn usage footer pushed by `drain_stream`: the
//! cumulative session totals and the context-window utilisation must be
//! visible without a slash command, and every unknown must stay absent.
//!
//! The turn is driven through the real seam — messages on the stream channel
//! drained by `drain_stream` — so the footer wording, the session totals, the
//! numerator capture, and the reset between turns are all exercised together.

use super::super::super::agent::Stream;
use super::super::tests_support::{idle_app, unused_runtime};
use super::*;
use saya_agent::{AgentOutput, CancellationToken, TokenUsage, UsageCall};
use std::sync::Arc;
use tokio::sync::mpsc::unbounded_channel;

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
fn run_turn(app: &mut App, state: &mut SessionState, messages: Vec<StreamMsg>) -> String {
    app.request.stream = Some(stream_with(messages));
    assert!(app.drain_stream(state), "the turn finished");
    app.transcript
        .blocks()
        .last()
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

// ---------------------------------------------------------------------------
// Adversarial additions: boundaries and orderings the tests above do not
// reach. Each pins a behaviour whose silent regression would mislead the
// reader of the footer rather than fail loudly.
// ---------------------------------------------------------------------------

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
