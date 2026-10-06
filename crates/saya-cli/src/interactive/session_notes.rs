//! `/notes`: the user's own notes on this session's continuation record.
//!
//! One shared operation for every surface: the slash parser translates the
//! line once, and both loops hand the parsed [`NotesCommand`] to
//! [`run_notes`]. Show reads the notes out, `set` stores them on the typed
//! record only, `clear` removes them. Notes are USER guidance attached to
//! this session's saved summary — never created by the model, never an
//! authority or a grant (decisions 3/17), and separate from project
//! preferences and from database knowledge. A record exists only after a
//! compaction, so `set` and `clear` refuse until one does; an
//! `Unsupported` (newer-binary) record is never touched (decision 22).
//!
//! Text is validated with the contract's own rule and refused up front
//! when the redactor would rewrite it: assemble drops such notes silently,
//! so this command must never claim it stored one.

use crate::interactive::session_state::SessionState;
use saya_store::StoredContinuation;
use saya_types::{ContinuationError, MAX_USER_NOTES_BYTES, redact};

/// What one `/notes` invocation asked for. Parsed once per surface; the
/// `Set` text arrives verbatim from the parse (outer ends trimmed,
/// interior spacing kept). Nominally `pub` like the crate-private `cli`
/// command types it sits beside: `SlashCommand` carries it, and the
/// enclosing module keeps the effective visibility crate-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotesCommand {
    Show,
    Set(String),
    Clear,
}

/// The header every shown notes text carries: the notes are the user's own
/// guidance and grant nothing.
pub(crate) const SHOW_HEADER: &str = "Session notes (yours; guidance only — they grant nothing):";

const NO_NOTES: &str = "No session notes. Add some with /notes set <text>.";
const STORED: &str = "Session notes stored.";
const CLEARED: &str = "Session notes cleared.";
const NO_RECORD: &str =
    "Notes attach to the session's saved summary, which /compact creates; run /compact first.";
const UNSUPPORTED_EDIT: &str =
    "This session was written by a newer version; its summary cannot be edited here.";
const UNSUPPORTED_SHOW: &str =
    "This session was written by a newer version; its notes cannot be shown here.";
const SECRET: &str = "that text looks like a secret or credential and was not stored";

/// The one `/notes` behaviour: mutates `state` in place, and answers with
/// `(ok, message)` — `ok` is the Message/Error split every surface renders
/// (and, headless, the turn's completed/errored outcome).
pub(crate) fn run_notes(state: &mut SessionState, command: NotesCommand) -> (bool, String) {
    match command {
        NotesCommand::Show => show_notes(state),
        NotesCommand::Set(text) => set_notes(state, text),
        NotesCommand::Clear => clear_notes(state),
    }
}

/// The show never refuses and never interprets an unsupported record.
fn show_notes(state: &SessionState) -> (bool, String) {
    match &state.continuation {
        None => (true, NO_NOTES.into()),
        Some(StoredContinuation::Current(record)) => match &record.user_notes {
            Some(notes) => (true, format!("{SHOW_HEADER}\n{notes}")),
            None => (true, NO_NOTES.into()),
        },
        Some(StoredContinuation::Unsupported(_)) => (true, UNSUPPORTED_SHOW.into()),
    }
}

/// Stores the notes on the typed record — and only there: narrative, facts,
/// and the source boundary are untouched. A refusal leaves the previous
/// notes exactly as they were.
fn set_notes(state: &mut SessionState, text: String) -> (bool, String) {
    let Some(StoredContinuation::Current(record)) = state.continuation.as_mut() else {
        return record_refusal(state);
    };
    // A note the redactor would rewrite cannot be stored honestly:
    // prepare_for_persist would rewrite it on disk, and a later compaction's
    // assemble would drop it silently — refuse before ever claiming success.
    if redact(&text) != text {
        return (false, SECRET.into());
    }
    let mut candidate = record.clone();
    candidate.user_notes = Some(text);
    if let Err(error) = candidate.validate() {
        return (false, notes_refusal(&error));
    }
    record.user_notes = candidate.user_notes;
    (true, STORED.into())
}

/// Removes the notes. Removal cannot invalidate the record, so no
/// revalidation is owed.
fn clear_notes(state: &mut SessionState) -> (bool, String) {
    let Some(StoredContinuation::Current(record)) = state.continuation.as_mut() else {
        return record_refusal(state);
    };
    record.user_notes = None;
    (true, CLEARED.into())
}

/// `set` and `clear` need a typed record to edit: none is the
/// /compact-first remedy, and an `Unsupported` record is a newer binary's
/// data — refused verbatim, never touched.
fn record_refusal(state: &SessionState) -> (bool, String) {
    match &state.continuation {
        Some(StoredContinuation::Unsupported(_)) => (false, UNSUPPORTED_EDIT.into()),
        _ => (false, NO_RECORD.into()),
    }
}

/// Maps the record's own validation errors to sentences. The record was
/// valid before the edit, so a notes-shaped failure is the only expected
/// kind; anything else falls through to the contract's own wording.
fn notes_refusal(error: &ContinuationError) -> String {
    match error {
        ContinuationError::UserNotesEmpty => "give the notes some text: /notes set <text>".into(),
        ContinuationError::UserNotesOversize(len) => {
            format!("notes are {len} bytes; the limit is {MAX_USER_NOTES_BYTES}")
        }
        ContinuationError::UserNotesControlCharacter => {
            "notes may contain only text, newlines and tabs".into()
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
#[path = "session_notes_tests.rs"]
mod session_notes_tests;
