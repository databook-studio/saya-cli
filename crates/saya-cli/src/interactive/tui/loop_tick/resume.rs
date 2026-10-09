//! Adopting a session-picker resume choice.

use super::super::session_save::queue_session_save;
use super::super::transcript::BlockKind;
use super::super::types::App;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::lifecycle::resume as session_resume;
use crate::interactive::session::permissions::activation as session_activation;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;
use saya_store::FsSessionStore;

/// A successful resume is a context change: the previous session's ephemeral
/// SQL capture and last query are invalidated with it, along with the
/// agent-capture gap that names the old session's promoted query (the spec
/// rule that a context change retires the old query/capture/receipt).
/// Refusals and failed loads change nothing — the old capture stays usable.
pub(crate) fn clear_query_state(app: &mut App) {
    app.captured = None;
    app.last_query = None;
    app.agent_captures.clear_gap();
}

/// The shared tail of a successful resume (picker or typed): the old
/// session's query state retires with it, the header facts are re-derived
/// from the resumed session's universe (`reacquire` recomposed it; the
/// swapped-in state arrived with the `#[serde(skip)]` defaults), the app
/// adopts the resumed session's universe (seeded with the task list), and
/// the runtime metrics collector is re-taken — `reacquire` replaced the
/// runtime, so the startup handle would otherwise keep counting into the
/// retired collector.
pub(crate) fn adopt_resumed_session(
    app: &mut App,
    state: &mut SessionState,
    session: &SessionRuntime,
) {
    clear_query_state(app);
    state.sync_host_posture(&session.universe());
    app.session = session.universe();
    app.session.seed_tasks(state.task_list.clone());
    app.session_metrics = session.session_metrics();
}

pub(crate) fn tick_resume(
    app: &mut App,
    store: &FsSessionStore,
    state: &mut SessionState,
    runtime: &RuntimeConfig,
    session: &mut SessionRuntime,
) {
    if let Some(id) = app.overlays.pending_resume.take() {
        // A resume swaps the session runtime: never under a running agent
        // stream, query or compaction (their late completion would settle
        // into the resumed session). Refused, nothing changes.
        if app.is_busy() {
            app.transcript.push(
                BlockKind::System,
                "A request is still running - stop it (Esc) or wait for it to finish, then resume again.",
            );
            return;
        }
        let defaults = session_resume::SessionDefaults {
            provider: state.provider.clone(),
            model: state.model.clone(),
            allow_data_sharing: state.allow_data_sharing,
            approval_mode: state.approval_mode.clone(),
        };
        match session_resume::resume_session(store, &id, &defaults) {
            Ok(Some(loaded)) => {
                // The resumed session's policy is its own, built from the
                // resumed mode: grants are process-lifetime facts about
                // one session, and a resumed session starts empty.
                match session.reacquire(
                    runtime,
                    loaded.workspace_root.as_deref(),
                    &id,
                    loaded
                        .approval_mode
                        .parse()
                        .unwrap_or(saya_agent::ApprovalPolicy::Ask),
                ) {
                    Ok(()) => {
                        session_resume::adopt_picker_resumed(
                            state,
                            loaded,
                            runtime.resolved.ai.base_url.as_deref(),
                        );
                        clear_query_state(app);
                        adopt_resumed_session(app, state, session);
                        app.reload_at_refs(state);
                        if state.turns.is_empty() {
                            app.transcript.clear();
                            app.transcript.push(
                                BlockKind::System,
                                format!("Resumed session {id} (no earlier turns)."),
                            );
                        } else {
                            // Replace the panel with the resumed session's conversation.
                            app.show_history(state);
                        }
                        if let Some(notice) = session.notice() {
                            app.transcript.push(BlockKind::System, notice.to_string());
                        }
                        // A resumed bypass session re-prints its
                        // activation line: the mode is real again.
                        if let Some(line) =
                            session_activation::line_if_bypass(state, runtime, &session.universe())
                        {
                            app.transcript.push(BlockKind::System, line);
                        }
                    }
                    Err(error) => app.transcript.push(BlockKind::Error, error),
                }
            }
            Ok(None) => app
                .transcript
                .push(BlockKind::Error, format!("Session not found: {id}")),
            Err(error) => app.transcript.push(BlockKind::Error, error.to_string()),
        }
        queue_session_save(&mut *app, store, state);
    }
}

#[cfg(test)]
#[path = "../../session_privacy_receipt_tests.rs"]
mod session_privacy_receipt_tests;

#[cfg(test)]
#[path = "../resume_capture_tests.rs"]
mod resume_capture_tests;

#[cfg(test)]
#[path = "../typed_resume_capture_tests.rs"]
mod typed_resume_capture_tests;
