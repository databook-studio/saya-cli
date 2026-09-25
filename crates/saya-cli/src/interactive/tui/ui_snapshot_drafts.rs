use super::super::stream_events::apply_event;
use super::super::transcript::BlockKind;
/// Composed-screen behaviour snapshots (draft marking).
/// Moved byte-identical from the hub; plain assertions only.
use super::support::{busy_stream, empty_app, fixed_status, render_buffer};
use saya_agent::AgentEvent;

/// Objective B: an assistant block in the live chapter while
/// `request.stream.is_some()` is a draft — the label row says so in a plain
/// word, and `block.text` is untouched so clipboard copy is unaffected.
#[test]
fn a_streaming_answer_is_marked_a_draft() {
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "count the orders");
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text("the total is 42"),
        false,
    );
    app.request.stream = Some(busy_stream());
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("SAYA (draft)"),
        "a streaming answer's label must say it is a draft:\n{buffer}"
    );
}

/// Objective B: once the turn ends (`Done` clears the stream), the same block
/// renders as `SAYA` again — the marking follows the state automatically.
#[test]
fn a_finished_answer_is_not_marked_a_draft() {
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "count the orders");
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text("the total is 42"),
        false,
    );
    assert!(
        app.request.stream.is_none(),
        "no stream: the turn is finished"
    );
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(
        buffer.contains("SAYA"),
        "a finished answer keeps its SAYA label:\n{buffer}"
    );
    assert!(
        !buffer.contains("SAYA (draft)"),
        "a finished answer must not say draft:\n{buffer}"
    );
}

/// Objective B: only the live chapter's assistant block is a draft. An
/// earlier chapter — even while a later turn streams — renders exactly as
/// today, with no draft wording anywhere near it.
#[test]
fn an_earlier_chapters_answer_is_never_marked_a_draft() {
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "first question");
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text("first answer"),
        false,
    );
    app.transcript.push(BlockKind::User, "second question");
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text("second answer so far"),
        false,
    );
    app.request.stream = Some(busy_stream());
    let buffer = render_buffer(&app, &fixed_status(), 100, 30);
    assert_eq!(
        buffer.matches("SAYA (draft)").count(),
        1,
        "exactly one draft marking — the live chapter's — may render:\n{buffer}"
    );
    assert!(
        buffer.contains("first answer"),
        "the earlier chapter's answer is still on screen:\n{buffer}"
    );
}

/// Objective B: marking a draft must not change `block.text`, so what copy
/// yields is byte-identical whether the turn is streaming or finished.
#[test]
fn marking_a_draft_does_not_change_what_copy_yields() {
    let mut app = empty_app();
    app.transcript.push(BlockKind::User, "count the orders");
    apply_event(
        &mut app.transcript,
        AgentEvent::assistant_text("the total is 42"),
        false,
    );
    app.copy_last_answer();
    let finished_copy = app.pending_clipboard.clone().expect("answer was queued");
    app.pending_clipboard = None;

    let mut streaming = empty_app();
    streaming
        .transcript
        .push(BlockKind::User, "count the orders");
    apply_event(
        &mut streaming.transcript,
        AgentEvent::assistant_text("the total is 42"),
        false,
    );
    streaming.request.stream = Some(busy_stream());
    streaming.copy_last_answer();
    let streaming_copy = streaming
        .pending_clipboard
        .clone()
        .expect("streaming answer was queued");
    assert_eq!(
        streaming_copy, finished_copy,
        "the draft marking must not change what copy yields"
    );
    assert_eq!(streaming_copy, "the total is 42");
}
