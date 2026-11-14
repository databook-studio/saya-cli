use super::*;

/// With no continuation record, `set` and `clear` refuse with the
/// /compact-first remedy — a record exists only after a compaction.
#[test]
fn set_and_clear_without_a_record_refuse_with_the_compact_remedy() {
    let refusal = "Notes attach to the session's saved summary, which /compact \
                   creates; run /compact first.";
    let mut state = SessionState::new("s1", None, "m");
    let (ok, message) = run_notes(&mut state, NotesCommand::Set("text".into()));
    assert!(!ok);
    assert_eq!(message, refusal);
    let (ok, message) = run_notes(&mut state, NotesCommand::Clear);
    assert!(!ok);
    assert_eq!(message, refusal);
    assert!(state.continuation.is_none(), "nothing was created");
}

/// An `Unsupported` record is a newer binary's data: `set` and `clear`
/// refuse and the raw value is byte-identical afterwards.
#[test]
fn an_unsupported_record_is_never_touched() {
    let raw = serde_json::json!({"version": 2, "future": true});
    let mut state = SessionState::new("s1", None, "m");
    state.continuation = Some(StoredContinuation::Unsupported(raw.clone()));
    for command in [NotesCommand::Set("text".into()), NotesCommand::Clear] {
        let (ok, message) = run_notes(&mut state, command);
        assert!(!ok, "an unsupported record is refused: {message}");
        assert!(
            message.contains("newer version") && message.contains("cannot be edited"),
            "the refusal names the newer-binary record: {message}"
        );
        assert_eq!(
            state.continuation,
            Some(StoredContinuation::Unsupported(raw.clone())),
            "the newer binary's record is byte-identical"
        );
    }
}

/// A note over the byte cap is refused with the bound named, and the
/// previous notes survive unchanged.
#[test]
fn an_oversize_note_is_refused_and_previous_notes_survive() {
    let mut state = state_with_current(Some("keep"));
    let oversize = "x".repeat(saya_types::MAX_USER_NOTES_BYTES + 1);
    let (ok, message) = run_notes(&mut state, NotesCommand::Set(oversize));
    assert!(!ok);
    assert_eq!(
        message, "notes are 8193 bytes; the limit is 8192",
        "the refusal names the bound"
    );
    assert_eq!(
        current(&state).user_notes.as_deref(),
        Some("keep"),
        "the previous notes survive the refusal"
    );
}

/// An empty note is refused with the remedy.
#[test]
fn an_empty_note_is_refused() {
    let mut state = state_with_current(Some("keep"));
    let (ok, message) = run_notes(&mut state, NotesCommand::Set(String::new()));
    assert!(!ok);
    assert_eq!(message, "give the notes some text: /notes set <text>");
    assert_eq!(current(&state).user_notes.as_deref(), Some("keep"));
}

/// A control character other than newline and tab is refused, and the
/// previous notes survive.
#[test]
fn a_control_character_is_refused() {
    let mut state = state_with_current(Some("keep"));
    let (ok, message) = run_notes(&mut state, NotesCommand::Set("bad\u{7}text".into()));
    assert!(!ok);
    assert_eq!(message, "notes may contain only text, newlines and tabs");
    assert_eq!(current(&state).user_notes.as_deref(), Some("keep"));
}

/// Text the redactor would rewrite is refused up front — assemble would
/// drop it silently, so the command must never claim it stored.
#[test]
fn a_secret_shaped_note_is_refused_and_previous_notes_survive() {
    let text = "api_key=abc123";
    assert_ne!(redact(text), text, "the fixture is secret-shaped");
    let mut state = state_with_current(Some("keep"));
    let (ok, message) = run_notes(&mut state, NotesCommand::Set(text.into()));
    assert!(!ok);
    assert_eq!(
        message,
        "that text looks like a secret or credential and was not stored"
    );
    assert_eq!(
        current(&state).user_notes.as_deref(),
        Some("keep"),
        "the previous notes survive the refusal"
    );
}
