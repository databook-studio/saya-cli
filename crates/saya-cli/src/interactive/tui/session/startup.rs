//! Building the `App` and seeding the transcript before the first draw.

use super::super::transcript::BlockKind;
use super::super::types::{App, TrustPrompt};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_activation;
use crate::interactive::session_state::SessionState;
use crate::interactive::session_trust;
use saya_store::SqliteStateStore;
use std::sync::Arc;

/// Builds the `App` for the session: profiles, notices, resumed history —
/// and arms the startup trust modal once the splash has painted (never
/// before it).
pub(crate) fn build_app(
    runtime: &RuntimeConfig,
    state_db: &SqliteStateStore,
    state: &mut SessionState,
    session: &mut crate::interactive::session_runtime::SessionRuntime,
    trusted_echo: Option<&str>,
    trust_pending: bool,
) -> App {
    let profiles = runtime
        .connections
        .profiles
        .keys()
        .cloned()
        .collect::<Vec<String>>();
    let mut app = App::new(
        profiles,
        Arc::new(runtime.clone()),
        state_db.clone(),
        session.universe(),
    );
    // A startup fact the user must read: a pinned root that vanished, or any
    // other composition notice, said once into the transcript — the trust
    // answer's echo beside it where this launch trusted a folder — and,
    // under bypass, the mode's activation line with the probe/absence facts.
    if let Some(notice) = session.notice() {
        app.transcript.push(BlockKind::System, notice.to_string());
    }
    if let Some(echo) = trusted_echo {
        app.transcript.push(BlockKind::System, echo.to_string());
    }
    if let Some(line) = session_activation::line_if_bypass(state, runtime, &session.universe()) {
        app.transcript.push(BlockKind::System, line);
    }
    // Bypass × unbound × non-terminal cannot reach the TUI (the TUI needs a
    // terminal), but the headless loop's twin below keeps the one wording —
    // `bypass_no_lane_note` — so the two surfaces cannot drift.
    if let Some(note) = session_trust::bypass_no_lane_note(
        session_activation::is_bypass_mode(state),
        session.universe().host_composed(),
    ) {
        app.transcript.push(BlockKind::System, note);
    }
    app.reload_at_refs(state);
    // A session resumed via --resume/--continue arrives with its turns already
    // loaded; replay them so the panel opens on the prior conversation.
    app.show_history(state);
    // The startup trust modal opens after the splash paints — never before
    // it — so the PTY splash assertion holds on every launch and the
    // question is answered inside the interface, not in front of it.
    if trust_pending {
        app.overlays.trust = Some(TrustPrompt::default());
    }
    app
}
