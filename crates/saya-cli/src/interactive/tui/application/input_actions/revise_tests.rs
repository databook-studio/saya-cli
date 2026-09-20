//! Stop and revise: a prompt submitted after a stop waits for the
//! confirmation, and says so.

use super::*;
use crate::interactive::tui::application::tests_support::idle_app;

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
