//! Adopting a session-picker resume choice.

use super::super::session_save::queue_session_save;
use super::super::transcript::BlockKind;
use super::super::types::App;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_activation;
use crate::interactive::session_resume;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use saya_store::FsSessionStore;

pub(crate) fn tick_resume(
    app: &mut App,
    store: &FsSessionStore,
    state: &mut SessionState,
    runtime: &RuntimeConfig,
    session: &mut SessionRuntime,
) {
    if let Some(id) = app.overlays.pending_resume.take() {
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
                        crate::interactive::adopt_picker_resumed(
                            state,
                            loaded,
                            runtime.resolved.ai.base_url.as_deref(),
                        );
                        app.session = session.universe();
                        app.session.seed_tasks(state.task_list.clone());
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
