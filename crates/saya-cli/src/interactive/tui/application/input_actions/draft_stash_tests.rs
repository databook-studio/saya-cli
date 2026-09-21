//! History recall and the in-progress draft it must not eat.

use super::tests::{app_with_history, type_draft};
use super::*;
use crate::interactive::tui::application::tests_support::idle_app;
use crate::interactive::tui::history::History;

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
