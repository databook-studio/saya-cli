use super::*;

/// `/notes` with no continuation record says there are no notes and names
/// the remedy — a record exists only after a compaction.
#[test]
fn show_without_a_record_says_there_are_none() {
    let mut state = SessionState::new("s1", None, "m");
    let (ok, message) = run_notes(&mut state, NotesCommand::Show);
    assert!(ok, "a show is never a refusal");
    assert_eq!(
        message,
        "No session notes. Add some with /notes set <text>."
    );
}

/// `/notes` over a record whose notes are absent says the same thing.
#[test]
fn show_with_a_record_without_notes_says_there_are_none() {
    let mut state = state_with_current(None);
    let (ok, message) = run_notes(&mut state, NotesCommand::Show);
    assert!(ok);
    assert_eq!(
        message,
        "No session notes. Add some with /notes set <text>."
    );
}

/// `/notes` renders the notes under the one header line, verbatim.
#[test]
fn show_renders_the_header_and_the_notes() {
    let mut state = state_with_current(Some("keep these\nnotes"));
    let (ok, message) = run_notes(&mut state, NotesCommand::Show);
    assert!(ok);
    assert_eq!(
        message,
        format!(
            "{}\nkeep these\nnotes",
            "Session notes (yours; guidance only — they grant nothing):"
        )
    );
}

/// `/notes` over an `Unsupported` record says the newer-binary facts: the
/// record is never interpreted to show its notes.
#[test]
fn show_on_an_unsupported_record_says_it_cannot_be_shown() {
    let mut state = SessionState::new("s1", None, "m");
    let raw = serde_json::json!({"version": 2, "user_notes": "future notes"});
    state.continuation = Some(StoredContinuation::Unsupported(raw));
    let (ok, message) = run_notes(&mut state, NotesCommand::Show);
    assert!(ok, "a show reports; it does not refuse");
    assert!(
        message.contains("newer version") && message.contains("cannot be shown"),
        "the show names the newer-binary record: {message}"
    );
}
