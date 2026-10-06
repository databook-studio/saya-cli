//! Tests for the shared `/notes` operation: what it shows, what it stores,
//! what it refuses, and how the updated record round-trips through the
//! store and a later compaction.

use super::*;
use crate::interactive::session_compact::{apply_with_continuation, plan as compact_plan};
use crate::interactive::session_resume::{SessionDefaults, block_on, state_from_redacted};
use crate::interactive::session_state::SessionState;
use saya_store::{FsSessionStore, SessionStore, StoredContinuation};
use saya_types::{CONTINUATION_VERSION, SessionContinuationV1, SourceBoundary, redact};

fn seeded_current(notes: Option<&str>) -> SessionContinuationV1 {
    let record = SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: SourceBoundary {
            through_turn: 1,
            source_sha256: "b".repeat(64),
            workspace_root: None,
            profile: None,
        },
        narrative: None,
        user_notes: notes.map(str::to_owned),
        files: Vec::new(),
        checks: Vec::new(),
    };
    record.validate().expect("the seeded record validates");
    record
}

fn state_with_current(notes: Option<&str>) -> SessionState {
    let mut state = SessionState::new("s1", None, "m");
    state.continuation = Some(StoredContinuation::Current(seeded_current(notes)));
    state
}

fn current(state: &SessionState) -> &SessionContinuationV1 {
    match &state.continuation {
        Some(StoredContinuation::Current(record)) => record,
        other => panic!("expected a typed continuation, found {other:?}"),
    }
}

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

/// A stored note round-trips through a real `FsSessionStore`: the session
/// is saved with the notes on the record, and a resume reads them back.
#[test]
fn the_notes_round_trip_through_a_real_store() {
    let root = std::env::temp_dir().join(format!(
        "saya-notes-roundtrip-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create the store root");
    let store = FsSessionStore::new(&root);
    let mut state = state_with_current(None);
    let (ok, message) = run_notes(&mut state, NotesCommand::Set("persist these".into()));
    assert!(ok, "{message}");
    block_on(store.save(state.redacted())).expect("the record saves");
    let loaded = block_on(store.load("s1"))
        .expect("loads")
        .expect("the record is there");
    let resumed = state_from_redacted(
        loaded,
        &SessionDefaults {
            provider: "ollama".into(),
            model: "m".into(),
            allow_data_sharing: false,
            approval_mode: "ask".into(),
        },
    );
    assert_eq!(
        current(&resumed).user_notes.as_deref(),
        Some("persist these"),
        "the notes ride the store and the resume"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// A compaction after `set` keeps the notes: they ride the new record,
/// preserved apart from the generated narrative.
#[test]
fn a_compaction_after_set_keeps_the_notes() {
    let mut state = state_with_current(None);
    for index in 0..5 {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    let (ok, message) = run_notes(&mut state, NotesCommand::Set("keep these".into()));
    assert!(ok, "{message}");
    let plan = compact_plan(&state.turns, &state.provider_history()).expect("a long history plans");
    apply_with_continuation(&mut state, &plan, "older turns discussed.", &|_| None)
        .expect("applies");
    assert_eq!(
        current(&state).user_notes.as_deref(),
        Some("keep these"),
        "the notes ride the new record across the compaction"
    );
    assert_eq!(
        current(&state).source.through_turn,
        5,
        "the boundary is the new compaction's"
    );
}
