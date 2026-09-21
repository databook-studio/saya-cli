//! Phase 8 packet 2: the key that returns a scrolled reader to the live edge.
//!
//! `Shift+End` is otherwise unbound: bare `End` keeps moving the input
//! cursor, and the arm sits after the approval and Esc blocks, so it can
//! neither answer a modal nor reorder the stop paths.

use super::*;
use crate::interactive::tui::application::tests_support::idle_app;
use crate::interactive::tui::transcript::BlockKind;
use crate::interactive::tui::types::PendingApproval;

/// A scrolled-up app with a stored view and one row below the fold.
fn app_with_new_activity() -> App {
    let mut app = idle_app();
    for i in 0..8 {
        app.transcript
            .push(BlockKind::Assistant, format!("line {i}"));
    }
    app.transcript.scroll_up(2, 80, 4);
    let _ = app.transcript.view(80, 4);
    app.transcript
        .push(BlockKind::Assistant, "landed below the fold");
    app
}

#[test]
fn the_return_key_returns_to_the_live_edge() {
    let mut app = app_with_new_activity();
    assert!(
        app.unseen_new_rows() > 0,
        "precondition: rows arrived below"
    );
    handle_key(&mut app, KeyCode::End, KeyModifiers::SHIFT);
    assert!(
        app.transcript.is_following_tail(),
        "Shift+End returns the reader to the live edge"
    );
    assert_eq!(app.unseen_new_rows(), 0, "and clears the count");
}

#[test]
fn the_return_key_does_not_disturb_an_approval() {
    let mut app = app_with_new_activity();
    let (respond, mut answer) = tokio::sync::oneshot::channel();
    app.request.pending_approval = Some(PendingApproval {
        tool: "bounded_sql_query".into(),
        detail: Some("SELECT 1".into()),
        grant: None,
        respond,
    });
    let before = app.unseen_new_rows();
    handle_key(&mut app, KeyCode::End, KeyModifiers::SHIFT);
    assert!(
        app.request.pending_approval.is_some(),
        "Shift+End must not answer the approval modal"
    );
    assert!(
        answer.try_recv().is_err(),
        "no approval answer may be sent by the return key"
    );
    assert_eq!(
        app.unseen_new_rows(),
        before,
        "the modal swallows the key; it is not a jump"
    );
}

#[test]
fn bare_end_still_moves_the_input_cursor() {
    let mut app = idle_app();
    app.input.set_text("hello");
    app.input.move_home();
    handle_key(&mut app, KeyCode::End, KeyModifiers::NONE);
    assert!(
        app.input.cursor() > 0,
        "bare End is still end-of-line, never go-to-bottom"
    );
}
