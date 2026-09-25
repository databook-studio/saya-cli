//! Phase 8 packet 2: the App-facing "new activity" view state.
//!
//! The counter is presentation state on the transcript: returning to the live
//! edge clears it, and it never reaches `SessionState` or anything the model
//! replays. Beside `new_activity.rs`, the module that exposes it to the app.

use crate::interactive::session_state::SessionState;
use crate::interactive::tui::application::tests_support::idle_app;
use crate::interactive::tui::transcript::BlockKind;
use crate::interactive::tui::types::App;

/// An app whose reader has scrolled up, with a stored view, and one row that
/// has landed below the fold — the shape the indicator renders from.
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
fn returning_to_the_live_edge_clears_the_count() {
    let mut app = app_with_new_activity();
    assert!(
        app.unseen_new_rows() > 0,
        "precondition: rows arrived below"
    );
    assert!(!app.transcript.is_following_tail());
    app.return_to_live_edge();
    assert_eq!(
        app.unseen_new_rows(),
        0,
        "reaching the tail clears the count"
    );
    assert!(
        app.transcript.is_following_tail(),
        "returning to the live edge is entering follow mode"
    );
}

#[test]
fn the_count_never_reaches_session_state() {
    let app = app_with_new_activity();
    assert!(
        app.unseen_new_rows() > 0,
        "precondition: the view carries a live count"
    );

    let mut state = SessionState::new("s1", Some("analytics".into()), "m");
    state.record_turn("what is below", "twelve new lines", false, Vec::new());
    let recorded = serde_json::to_string(&state).expect("a session serializes");
    for leaked in ["unseen", "new_lines", "new_activity"] {
        assert!(
            !recorded.contains(leaked),
            "record_turn must never write the view counter; found {leaked} in {recorded}"
        );
    }
    let redacted = serde_json::to_value(state.redacted()).expect("a redacted session serializes");
    for key in ["unseen", "unseen_new_rows", "new_lines", "new_activity"] {
        assert!(
            redacted.get(key).is_none(),
            "session state must not carry the view counter: {key}"
        );
    }
}
