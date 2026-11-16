use super::*;

/// `set` stores the notes and touches nothing else: narrative, facts, and
/// the source boundary survive untouched, and the record still validates.
#[test]
fn set_stores_the_notes_and_touches_nothing_else() {
    let mut record = seeded_current(None);
    record.narrative = Some(saya_types::GeneratedNarrative {
        text: "the summary.".into(),
        source: saya_types::NarrativeSource::Summarizer,
    });
    let mut state = SessionState::new("s1", None, "m");
    state.continuation = Some(StoredContinuation::Current(record));
    let before = current(&state).clone();
    let (ok, message) = run_notes(&mut state, NotesCommand::Set("keep these".into()));
    assert!(ok, "a clean set stores: {message}");
    assert_eq!(message, "Session notes stored.");
    let after = current(&state);
    assert_eq!(after.user_notes.as_deref(), Some("keep these"));
    assert_eq!(after.narrative, before.narrative, "narrative untouched");
    assert_eq!(after.files, before.files, "facts untouched");
    assert_eq!(after.source, before.source, "boundary untouched");
    after.validate().expect("the updated record validates");
}

/// `set` replaces the previous notes.
#[test]
fn set_replaces_previous_notes() {
    let mut state = state_with_current(Some("old"));
    let (ok, _) = run_notes(&mut state, NotesCommand::Set("new".into()));
    assert!(ok);
    assert_eq!(current(&state).user_notes.as_deref(), Some("new"));
}

/// `clear` removes the notes.
#[test]
fn clear_removes_the_notes() {
    let mut state = state_with_current(Some("old"));
    let (ok, message) = run_notes(&mut state, NotesCommand::Clear);
    assert!(ok, "a clear over notes completes: {message}");
    assert_eq!(message, "Session notes cleared.");
    assert_eq!(current(&state).user_notes, None);
}
