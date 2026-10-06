use saya_agent::ApprovalPolicy;
use saya_store::{
    FsSessionStore, MAX_SESSION_HISTORY_PAGE_SIZE, SessionHistoryQuery, SessionStore,
};

use super::transcript::{BlockKind, Transcript};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_resume::{SessionDefaults, block_on, resume_session};
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;

/// Lists saved sessions (shared with the `/history` command).
pub(super) fn list_sessions(transcript: &mut Transcript, store: &FsSessionStore) {
    match block_on(
        store.history(
            SessionHistoryQuery::first_page(MAX_SESSION_HISTORY_PAGE_SIZE)
                .expect("history page bound is valid"),
        ),
    ) {
        Ok(page) if page.entries.is_empty() => {
            transcript.push(BlockKind::System, "No saved sessions.")
        }
        Ok(page) => {
            let more = page.next_cursor.is_some();
            let mut body = page
                .entries
                .into_iter()
                .map(|entry| format!("{}\t{}", entry.id, entry.modified_unix_ms))
                .collect::<Vec<_>>()
                .join("\n");
            if more {
                body.push_str("\n(more saved sessions available)");
            }
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
                    // The header facts ride the NEW universe: `reacquire`
                    // recomposed it, and the swapped-in state arrived with
                    // the `#[serde(skip)]` defaults.
                    state.sync_host_posture(&session.universe());
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

/// Continues the session in a small successor: the shared steps run once in
/// `session_rotate_run` for both surfaces, so this adapter only renders the
/// outcome. The adoption tail is the typed `/resume` one — the successor's
/// notice and (under bypass) its activation line ride the swap — and the
/// app-level adoption (universe, metrics, query-state retirement) happens
/// through the loop's id-change check once this returns.
pub(super) fn rotate(
    transcript: &mut Transcript,
    state: &mut SessionState,
    runtime: &RuntimeConfig,
    store: &FsSessionStore,
    session: &mut SessionRuntime,
) {
    use crate::interactive::session_rotate_run::{RotateResult, rotate_session, rotated_message};
    match rotate_session(store, runtime, state, session) {
        RotateResult::Rotated { old, new } => {
            transcript.push(BlockKind::System, rotated_message(&old, &new));
            if let Some(notice) = session.notice() {
                transcript.push(BlockKind::System, notice.to_string());
            }
            // A bypass successor re-prints its activation line: the mode is
            // real again, in its own words.
            if let Some(line) = crate::interactive::session_activation::line_if_bypass(
                state,
                runtime,
                &session.universe(),
            ) {
                transcript.push(BlockKind::System, line);
            }
        }
        RotateResult::Refused(message) | RotateResult::Incomplete(message) => {
            transcript.push(BlockKind::Error, message);
        }
    }
}
