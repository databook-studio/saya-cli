use saya_agent::ApprovalPolicy;
use saya_store::{FsSessionStore, SessionStore};

use super::transcript::{BlockKind, Transcript};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_resume::{SessionDefaults, block_on, resume_session};
use crate::interactive::session_runtime::SessionRuntime;
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
/// session's settings for any fields the saved copy lacks. The engine side
/// rides the swap: the resumed session's own state directory is claimed and
/// composed before this one releases; a refused swap (a live holder, a
/// composition failure) keeps the current session and says why.
pub(super) fn resume(
    transcript: &mut Transcript,
    state: &mut SessionState,
    runtime: &RuntimeConfig,
    store: &FsSessionStore,
    session: &mut SessionRuntime,
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
            // The resumed session's policy is its own, built from the resumed
            // mode: grants are process-lifetime facts about one session, and
            // a resumed session starts empty.
            match session.reacquire(
                runtime,
                loaded.workspace_root.as_deref(),
                id,
                loaded.approval_mode.parse().unwrap_or(ApprovalPolicy::Ask),
            ) {
                Ok(()) => {
                    *state = loaded;
                    state.bind_runtime_endpoint(runtime.resolved.ai.base_url.as_deref());
                    transcript.push(BlockKind::System, format!("Resumed session {id}"));
                    if let Some(notice) = session.notice() {
                        transcript.push(BlockKind::System, notice.to_string());
                    }
                    // A resumed bypass session re-prints its activation line
                    // — the mode is real again, in its own words.
                    if let Some(line) = crate::interactive::session_activation::line_if_bypass(
                        state,
                        runtime,
                        &session.universe(),
                    ) {
                        transcript.push(BlockKind::System, line);
                    }
                }
                Err(error) => transcript.push(BlockKind::Error, error),
            }
        }
        Ok(None) => transcript.push(BlockKind::Error, format!("Session not found: {id}")),
        Err(error) => transcript.push(BlockKind::Error, error.to_string()),
    }
}
