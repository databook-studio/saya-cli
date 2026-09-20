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

/// A prompt queued after Esc says *why* it waits: the wording names the
/// stop confirmation, not just the current request. Phase 5 packet 4.
/// RED: `submit()` has no stop-aware branch yet.
#[test]
fn a_revision_after_a_stop_waits_for_the_confirmation() {
    use crate::interactive::tui::agent::Stream;
    use saya_agent::CancellationToken;
    let mut app = idle_app();
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    cancel.cancel();
    app.request.stream = Some(Stream {
        rx,
        cancel,
        prompt: "the first question".into(),
    });
    assert!(app.is_busy(), "precondition: the stream is still in flight");
    app.input.set_text("the revised question");
    app.submit();
    assert_eq!(
        app.pending.as_deref(),
        Some("the revised question"),
        "the revision is held, nothing dispatched"
    );
    assert!(
        app.request.stream.is_some(),
        "the stopped stream is still in flight — no dispatch ran"
    );
    let notice = app
        .transcript
        .blocks()
        .iter()
        .find(|b| b.text.contains("the revised question"))
        .expect("the queued notice quotes the revision");
    assert!(
        notice.text.contains("stop is confirmed") || notice.text.contains("stop confirmation"),
        "the notice must say it waits for the stop confirmation: {}",
        notice.text
    );
}

/// The stop-aware notice stays a `System` block and adds no `User` block:
/// Phase 3's rule holds for the revision too. Phase 5 packet 4.
/// RED: the branch does not exist yet.
#[test]
fn a_held_revision_is_never_a_user_block() {
    use crate::interactive::tui::agent::Stream;
    use saya_agent::CancellationToken;
    let mut app = idle_app();
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    cancel.cancel();
    app.request.stream = Some(Stream {
        rx,
        cancel,
        prompt: "the first question".into(),
    });
    app.input.set_text("the revised question");
    app.submit();
    let holding = app
        .transcript
        .blocks()
        .iter()
        .find(|b| b.text.contains("the revised question"))
        .expect("the queued notice quotes the revision");
    assert_eq!(
        holding.kind,
        BlockKind::System,
        "the stop-wait notice stays a System block, never a User block"
    );
    assert!(
        !app.transcript
            .blocks()
            .iter()
            .any(|b| b.kind == BlockKind::User),
        "queueing a revision adds no User block"
    );
}

/// Packet 2's Ctrl+G still clears a stop-held revision: the same one-slot
/// queue, not a new mechanism. Phase 5 packet 4.
/// RED: passes trivially today (nothing to hold), pins the behaviour.
#[test]
fn the_held_revision_is_still_droppable() {
    use crate::interactive::tui::agent::Stream;
    use crate::interactive::tui::keys::handle_key;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    use saya_agent::CancellationToken;
    let mut app = idle_app();
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let cancel = CancellationToken::new();
    cancel.cancel();
    app.request.stream = Some(Stream {
        rx,
        cancel,
        prompt: "the first question".into(),
    });
    app.input.set_text("the revised question");
    app.submit();
    assert!(app.pending.is_some(), "precondition: the revision is held");
    handle_key(&mut app, KeyCode::Char('g'), KeyModifiers::CONTROL);
    assert!(app.pending.is_none(), "Ctrl+G clears the held revision");
    assert!(
        app.request.stream.is_some(),
        "the stopped stream is untouched by the drop"
    );
}

/// Ctrl+G returns before the shared disarm, so it must disarm itself. Without
/// this, arming Ctrl+C then dropping a queue leaves the app one keystroke
/// from exiting with no second warning.
#[test]
fn dropping_the_queue_disarms_the_exit_prompt() {
    let mut app = idle_app();
    app.pending = Some("queued question".into());
    app.ctrl_c_armed = false;
    app.drop_queued_prompt();
    assert!(
        !app.ctrl_c_armed,
        "a queue drop leaves no armed exit behind"
    );
    assert!(app.pending.is_none(), "and the queue is cleared");
}
