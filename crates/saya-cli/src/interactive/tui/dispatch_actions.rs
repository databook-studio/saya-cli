use saya_store::{FsSessionStore, SessionStore};

use super::transcript::{BlockKind, Transcript};
use crate::interactive::session_resume::{SessionDefaults, block_on, resume_session};
use crate::interactive::session_state::SessionState;

/// Lists saved sessions (shared with the `/history` command).
pub(super) fn list_sessions(transcript: &mut Transcript, store: &FsSessionStore) {
    match block_on(store.history()) {
        Ok(entries) if entries.is_empty() => {
            transcript.push(BlockKind::System, "No saved sessions.")
        }
        Ok(entries) => {
            let body = entries
                .into_iter()
                .map(|entry| format!("{}\t{}", entry.id, entry.modified_unix_ms))
                .collect::<Vec<_>>()
                .join("\n");
            transcript.push(BlockKind::System, body);
        }
        Err(error) => transcript.push(BlockKind::Error, error.to_string()),
    }
}

/// Loads a saved session by id and makes it active, falling back to the current
/// session's settings for any fields the saved copy lacks.
pub(super) fn resume(
    transcript: &mut Transcript,
    state: &mut SessionState,
    store: &FsSessionStore,
    id: &str,
) {
    let defaults = SessionDefaults {
        provider: state.provider.clone(),
        model: state.model.clone(),
        allow_data_sharing: state.allow_data_sharing,
        approval_mode: state.approval_mode.clone(),
    };
    match resume_session(store, id, &defaults) {
        Ok(Some(loaded)) => {
            *state = loaded;
            transcript.push(BlockKind::System, format!("Resumed session {id}"));
        }
        Ok(None) => transcript.push(BlockKind::Error, format!("Session not found: {id}")),
        Err(error) => transcript.push(BlockKind::Error, error.to_string()),
    }
}
