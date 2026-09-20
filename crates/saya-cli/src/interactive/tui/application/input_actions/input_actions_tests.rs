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
