use super::*;

/// Newlines and tabs are accepted prose.
#[test]
fn newlines_and_tabs_are_accepted() {
    let mut state = state_with_current(None);
    let (ok, _) = run_notes(&mut state, NotesCommand::Set("a\nb\tc".into()));
    assert!(ok);
    assert_eq!(current(&state).user_notes.as_deref(), Some("a\nb\tc"));
}

/// The stored text is exactly what was given: interior spacing survives
/// (the parse trims only the outer ends).
#[test]
fn set_stores_the_text_verbatim() {
    let mut state = state_with_current(None);
    let (ok, _) = run_notes(&mut state, NotesCommand::Set("a  b   c".into()));
    assert!(ok);
    assert_eq!(current(&state).user_notes.as_deref(), Some("a  b   c"));
}
