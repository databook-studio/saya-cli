use crate::agent::runtime::PromptOverrides;
use crate::cli::ContractsCommand;
use crate::interactive::session::state::SessionState;
use crate::slash::SlashCommand;
use saya_agent::AgentOutput;
use saya_agent::ApprovalPolicy;
use saya_store::FsSessionStore;
use std::path::Path;

/// `/sessions list` — the shared saved-session listing for both surfaces.
/// `Ok` is the same text the headless `saya sessions list` prints (rows
/// newest first, pending deletions named separately, the storage root
/// last); `Err` is the failure message in the headless words.
pub(crate) fn sessions_list_text(
    store: &FsSessionStore,
    sessions_root: &Path,
) -> Result<String, String> {
    let listing = crate::interactive::session::lifecycle::resume::block_on(
        crate::commands::saved_sessions::list_sessions(store),
    )
    .map_err(|error| format!("could not list saved sessions: {error}"))?;
    Ok(crate::commands::saved_sessions::render_listing(
        &listing,
        crate::commands::saved_sessions::unix_ms_now(),
        sessions_root,
    ))
}

/// `/sessions delete <id>` — the shared delete outcome for both surfaces.
/// `true` when the delete completed (the REPL turn completes, the TUI
/// pushes a System block); `false` for every refusal or failure (the turn
/// errors, the TUI pushes an Error block). The message is the headless
/// `render_delete` wording, byte for byte. `current_session_id` is the
/// calling session's own id on both surfaces, so a session can never
/// delete itself underneath its owner.
pub(crate) fn sessions_delete_text(
    store: &FsSessionStore,
    sessions_root: &Path,
    id: &str,
    current_session_id: Option<&str>,
) -> (bool, String) {
    let outcome = crate::commands::saved_sessions::delete_saved_session(
        store,
        sessions_root,
        id,
        current_session_id,
    );
    let (code, message) = crate::commands::saved_sessions::render_delete(&outcome, id);
    (code == 0, message)
}

/// The headless refusal for `/setup`: names the surface it needs and the
/// command to run instead. Shared by the loop arm so the wording has one
/// home; the TUI never uses it (there, `/setup` really runs).
pub(crate) const SETUP_TUI_ONLY_ERROR: &str =
    "Guided setup needs the full-screen terminal. Run `saya setup` from your shell.";

include!("action.rs");

include!("apply.rs");

include!("state_text.rs");

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
