//! The one-slot prompt queue: what the notice shows, and dropping it.

use super::*;
use crate::interactive::tui::application::tests_support::idle_app;

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
