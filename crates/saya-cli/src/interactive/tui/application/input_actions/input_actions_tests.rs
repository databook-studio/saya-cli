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

// --- Fieldnotes Phase 9, packet 1: history navigation never eats the draft. -
//
// Up/Down recall fills the draft; it never submits it — and it must never
// destroy what the user was typing. Entering history (first Up from an unset
// cursor) stashes the in-progress draft; Down past the newest entry restores
// it. RED: `history_next` clears the line whenever the cursor is unset,
// which is every time the user has been typing.

use crate::interactive::tui::history::History;
use crate::interactive::tui::types::App;

/// An app whose history holds two entries with persistence off, so recall is
/// exercisable without touching the disk.
fn app_with_history() -> App {
    let mut app = idle_app();
    app.history = History::with_entries(&["select 1", "select 2"]);
    app
}

/// Types `text` one keystroke at a time through `handle_key`, exactly like
/// the user: every key ends any history navigation (dispatch resets the
/// history cursor after each edit).
fn type_draft(app: &mut App, text: &str) {
    use crate::interactive::tui::keys::handle_key;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    for c in text.chars() {
        handle_key(app, KeyCode::Char(c), KeyModifiers::NONE);
    }
}

/// Down at idle — a draft on the line, no history position — leaves the
/// draft alone. RED: `history_next` clears it today.
#[test]
fn down_at_idle_leaves_the_draft_alone() {
    use crate::interactive::tui::keys::handle_key;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let mut app = app_with_history();
    type_draft(&mut app, "draft note");
    handle_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(
        app.input.text(),
        "draft note",
        "Down with no history position never wipes the draft"
    );
}

/// Draft, Up (recalls the newest entry), Down (steps past the newest entry
/// back to the live edge): the original draft returns verbatim. RED today.
#[test]
fn stepping_up_then_back_down_restores_the_draft() {
    use crate::interactive::tui::keys::handle_key;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let mut app = app_with_history();
    type_draft(&mut app, "draft note");
    handle_key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(app.input.text(), "select 2", "precondition: Up recalled");
    handle_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(
        app.input.text(),
        "draft note",
        "stepping back down restores the stashed draft"
    );
}

/// The stash survives more than one step: Up Up, then Down Down.
#[test]
fn stepping_up_twice_then_down_twice_restores_the_draft() {
    use crate::interactive::tui::keys::handle_key;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let mut app = app_with_history();
    type_draft(&mut app, "draft note");
    handle_key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    handle_key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(app.input.text(), "select 1", "precondition: two Ups");
    handle_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(app.input.text(), "select 2", "Down steps forward");
    handle_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(
        app.input.text(),
        "draft note",
        "the stash survives multiple steps and restores verbatim"
    );
}

/// Recall fills the draft and never submits: nothing queued, nothing
/// dispatched, no User block, no request started.
#[test]
fn recall_never_submits() {
    use crate::interactive::tui::keys::handle_key;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let mut app = app_with_history();
    type_draft(&mut app, "draft note");
    handle_key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(app.input.text(), "select 2", "recall fills the draft");
    assert!(app.pending.is_none(), "recall dispatches nothing");
    assert!(app.request.stream.is_none(), "recall starts no request");
    assert!(
        !app.transcript
            .blocks()
            .iter()
            .any(|b| b.kind == BlockKind::User),
        "recall opens no turn in the transcript"
    );
}

/// After a submit the stash is gone: a later Down must not resurrect the
/// pre-submit draft. Pins the clear-on-submit rule (the stash does not exist
/// before the fix, so this guards the behaviour once it does).
#[test]
fn submitting_clears_the_stash() {
    use crate::interactive::tui::keys::handle_key;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let mut app = idle_app();
    app.history = History::with_entries(&["select 1"]);
    type_draft(&mut app, "my draft");
    handle_key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(app.input.text(), "select 1", "precondition: recalled");
    handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        app.pending.as_deref(),
        Some("select 1"),
        "precondition: the recalled entry was submitted"
    );
    handle_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    assert!(
        app.input.text().is_empty(),
        "a later Down does not resurrect the pre-submit draft: {:?}",
        app.input.text()
    );
}

/// The stash is composer view state, like the draft itself: nothing on
/// `SessionState`, nothing in anything `record_turn` writes.
#[test]
fn the_stash_never_reaches_session_state() {
    use crate::interactive::session_state::SessionState;
    use crate::interactive::tui::keys::handle_key;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let mut app = idle_app();
    app.history = History::with_entries(&["select 1"]);
    type_draft(&mut app, "the stashed draft words");
    handle_key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        app.pending.as_deref(),
        Some("select 1"),
        "precondition: a turn was submitted while a stash existed"
    );
    let mut state = SessionState::new("s1", Some("analytics".into()), "m");
    state.record_turn("the submitted prompt", "the answer", false, Vec::new());
    let recorded = serde_json::to_string(&state).expect("a session serializes");
    for leaked in ["stashed", "stash"] {
        assert!(
            !recorded.contains(leaked),
            "record_turn must never write the draft stash; found {leaked} in {recorded}"
        );
    }
    let redacted = serde_json::to_value(state.redacted()).expect("a redacted session serializes");
    for key in ["stash", "draft_stash", "history_stash"] {
        assert!(
            redacted.get(key).is_none(),
            "session state must not carry the draft stash: {key}"
        );
    }
}
