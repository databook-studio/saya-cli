use super::*;
use crate::interactive::tui::application::tests_support::idle_app;

/// Copying the transcript excludes the model's chain-of-thought. Reasoning
/// restates row values and column contents in prose, and the clipboard is a
/// channel off-screen — putting it on the system clipboard is a sharper
/// exposure than showing it to the person already reading the answer. The
/// user and assistant blocks are copied; the thinking block is not.
#[test]
fn copy_transcript_excludes_thinking_blocks() {
    let mut app = idle_app();
    app.transcript.push(BlockKind::User, "what is the answer");
    app.transcript.push(
        BlockKind::Thinking,
        "the secret chain-of-thought about row values",
    );
    app.transcript
        .push(BlockKind::Assistant, "the answer is 42");

    app.copy_transcript();
    let copied = app.pending_clipboard.expect("transcript was queued");
    assert!(
        copied.contains("the answer is 42"),
        "assistant text must be copied: {copied}"
    );
    assert!(
        copied.contains("what is the answer"),
        "user text must be copied: {copied}"
    );
    assert!(
        !copied.contains("the secret chain-of-thought about row values"),
        "thinking must not reach the clipboard: {copied}"
    );
}

#[test]
fn folding_does_not_change_what_copy_yields() {
    let mut app = idle_app();
    app.transcript.push(BlockKind::User, "count the red orders");
    app.transcript
        .push(BlockKind::Assistant, "the red orders total 42");
    app.transcript.push(BlockKind::User, "and the blue ones");
    app.copy_transcript();
    let plain = app.pending_clipboard.take().expect("transcript was queued");

    app.transcript.toggle_chapter(1);
    app.copy_transcript();
    let folded = app.pending_clipboard.expect("folded transcript was queued");
    assert_eq!(plain, folded, "a fold is a view, not a redaction");
    assert!(
        folded.contains("the red orders total 42"),
        "hidden rows still copy: {folded}"
    );
}

/// `copy_last_answer` finds the assistant block, not a thinking block, so the
/// chain-of-thought never reaches the clipboard even when it is the most
/// recent block.
#[test]
fn copy_last_answer_skips_thinking_blocks() {
    let mut app = idle_app();
    app.transcript
        .push(BlockKind::Assistant, "the answer is 42");
    app.transcript
        .push(BlockKind::Thinking, "the secret chain-of-thought");

    app.copy_last_answer();
    let copied = app.pending_clipboard.expect("answer was queued");
    assert_eq!(copied, "the answer is 42");
}

/// The copy keys filter thinking out, but selection mode hands the screen
/// to the terminal, whose drag-select cannot be filtered. When reasoning is
/// visible the notice has to say so — otherwise the narrower guarantee the
/// help text makes about `Ctrl+B` reads as a general one.
#[test]
fn selection_mode_says_the_terminal_can_copy_visible_thinking() {
    let mut app = idle_app();
    app.transcript
        .push(BlockKind::Thinking, "chain-of-thought about row values");
    app.toggle_selection_mode();

    let notice = app
        .transcript
        .blocks()
        .iter()
        .rev()
        .find(|b| b.kind == BlockKind::System)
        .expect("a selection-mode notice");
    assert!(
        notice.text.contains("Thinking is on screen"),
        "the notice must name the exposure while thinking is visible: {}",
        notice.text
    );
}

/// With no reasoning on screen there is nothing extra to warn about, and a
/// standing warning would train the user to ignore it.
#[test]
fn selection_mode_stays_quiet_about_thinking_when_none_is_shown() {
    let mut app = idle_app();
    app.transcript
        .push(BlockKind::Assistant, "the answer is 42");
    app.toggle_selection_mode();

    let notice = app
        .transcript
        .blocks()
        .iter()
        .rev()
        .find(|b| b.kind == BlockKind::System)
        .expect("a selection-mode notice");
    assert!(
        !notice.text.contains("Thinking"),
        "no thinking is shown, so the notice must not mention it: {}",
        notice.text
    );
}

// --- Fieldnotes phase 5, packet 2: the queue is visible and droppable. ------
//
// `submit()` while busy holds the prompt in `App.pending` with only a generic
// "Queued …" notice, so the user can never read back what is held. The queue
// notice must quote the prompt (truncated), stay a `System` block (a `User`
// block would read as the active task), and the queue must be droppable via
// Ctrl+G (`handle_key`) without touching the active request or the draft.

/// Drives `submit()` with `text` in the input box while the app is busy: the
/// busy path queues the prompt into `pending` with a `System` notice.
fn submit_while_busy(app: &mut crate::interactive::tui::types::App, text: &str) {
    use crate::interactive::tui::application::tests_support::in_flight_task;
    if !app.is_busy() {
        app.sql_task = Some(in_flight_task());
    }
    assert!(app.is_busy(), "precondition: the app is busy");
    app.input.set_text(text);
    app.submit();
}

/// A queued prompt's own words are visible in the transcript.
#[test]
fn a_queued_prompt_shows_its_own_words() {
    let mut app = idle_app();
    submit_while_busy(&mut app, "why did revenue fall last month?");
    assert!(
        app.transcript
            .blocks()
            .iter()
            .any(|b| b.text.contains("why did revenue fall last month?")),
        "the queue notice must quote the prompt: {:?}",
        app.transcript
            .blocks()
            .iter()
            .map(|b| b.text.clone())
            .collect::<Vec<_>>()
    );
}

/// The block holding the queued prompt is `System`, and no `User` block was
/// added — a `User` block opens a chapter and would read as the active task.
#[test]
fn a_queued_prompt_is_never_a_user_block() {
    let mut app = idle_app();
    let users_before = app
        .transcript
        .blocks()
        .iter()
        .filter(|b| b.kind == BlockKind::User)
        .count();
    submit_while_busy(&mut app, "why did revenue fall last month?");
    let holding = app
        .transcript
        .blocks()
        .iter()
        .find(|b| b.text.contains("why did revenue fall last month?"))
        .expect("the queued prompt is quoted somewhere");
    assert_eq!(
        holding.kind,
        BlockKind::System,
        "the queue notice stays a System block, never a User block"
    );
    assert_eq!(
        app.transcript
            .blocks()
            .iter()
            .filter(|b| b.kind == BlockKind::User)
            .count(),
        users_before,
        "queueing adds no User block"
    );
}

/// After Ctrl+G the queue is empty and the transcript says so.
#[test]
fn the_queued_prompt_can_be_dropped() {
    use crate::interactive::tui::keys::handle_key;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let mut app = idle_app();
    submit_while_busy(&mut app, "why did revenue fall last month?");
    assert!(app.pending.is_some(), "precondition: a prompt is queued");
    handle_key(&mut app, KeyCode::Char('g'), KeyModifiers::CONTROL);
    assert!(app.pending.is_none(), "dropping clears the queue");
    assert!(
        app.transcript
            .blocks()
            .iter()
            .any(|b| b.text.contains("Dropped the queued prompt.")),
        "the transcript says the queue was dropped"
    );
}

/// Dropping the queue leaves the active request running: the busy marker is
/// untouched and no cancellation was requested.
#[test]
fn dropping_the_queue_leaves_the_active_request_running() {
    use crate::interactive::tui::keys::handle_key;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let mut app = idle_app();
    submit_while_busy(&mut app, "why did revenue fall last month?");
    handle_key(&mut app, KeyCode::Char('g'), KeyModifiers::CONTROL);
    assert!(app.is_busy(), "the active request is still running");
    assert!(app.sql_task.is_some(), "the in-flight task is untouched");
}

/// Dropping the queue keeps whatever the user was typing.
#[test]
fn dropping_the_queue_keeps_the_input_draft() {
    use crate::interactive::tui::keys::handle_key;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let mut app = idle_app();
    submit_while_busy(&mut app, "why did revenue fall last month?");
    app.input.set_text("a fresh draft");
    handle_key(&mut app, KeyCode::Char('g'), KeyModifiers::CONTROL);
    assert_eq!(
        app.input.text(),
        "a fresh draft",
        "the composer draft survives the drop"
    );
}

/// Replacing a queued prompt keeps saying so, and shows the new text — never
/// the old one.
#[test]
fn replacing_a_queued_prompt_shows_the_new_text() {
    let mut app = idle_app();
    submit_while_busy(&mut app, "the stale question");
    submit_while_busy(&mut app, "the new question");
    assert_eq!(
        app.pending.as_deref(),
        Some("the new question"),
        "pending holds the full replacement text"
    );
    assert!(
        app.transcript
            .blocks()
            .iter()
            .any(|b| b.text.contains("replaced the earlier queued prompt")
                && b.text.contains("the new question")),
        "the replacement notice names the new prompt"
    );
    assert!(
        !app.transcript
            .blocks()
            .last()
            .expect("a notice was pushed")
            .text
            .contains("the stale question"),
        "the latest notice no longer shows the replaced text"
    );
}
