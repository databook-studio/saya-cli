use super::*;

// ---------------------------------------------------------------------------
// No control characters in either text field
// ---------------------------------------------------------------------------

#[test]
fn control_characters_are_refused_in_title_and_note() {
    let error = SessionTask::new("title\u{0007}bell", TaskStatus::Pending).unwrap_err();
    assert!(
        matches!(error, SessionTaskError::TitleControlCharacter),
        "a control char in the title refuses: {error:?}"
    );
    let error =
        SessionTask::with_note("task", TaskStatus::Pending, Some("note\nnewline")).unwrap_err();
    assert!(
        matches!(error, SessionTaskError::NoteControlCharacter),
        "a control char in the note refuses: {error:?}"
    );
    let error =
        SessionTask::with_note("task", TaskStatus::Pending, Some("note\u{0}nul")).unwrap_err();
    assert!(
        matches!(error, SessionTaskError::NoteControlCharacter),
        "a NUL in the note refuses: {error:?}"
    );
}

#[test]
fn validate_rejects_control_characters_that_skipped_the_constructor() {
    let list: SessionTaskList =
        serde_json::from_str(r#"{"tasks":[{"title":"ok","status":"pending","note":"a\rb"}]}"#)
            .unwrap();
    assert!(
        matches!(list.validate(), Err(SessionTaskError::NoteControlCharacter)),
        "validate() itself must catch the control char"
    );
}
