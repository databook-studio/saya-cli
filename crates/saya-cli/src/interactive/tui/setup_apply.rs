//! The post-setup apply (U07): when the guided setup closes, the TUI runs
//! the same reload `/reload` runs — one shared swap, one shared report —
//! and renders the close and its outcome in a single block. A wizard that
//! wrote nothing (its own cancel is exit 0) reloads into "unchanged",
//! which is correct and cheap. Any other outcome keeps the U05 blocks and
//! never reloads.

use std::io;
use std::sync::Arc;

use super::loop_tick::pending::apply_reloaded_runtime;
use super::terminal_handoff::SetupHandoff;
use super::transcript::BlockKind;
use super::types::App;
use crate::cli::GlobalOptions;
use crate::config::reload::{ReloadInputs, ReloadOutcome, reload_runtime};
use crate::interactive::session::state::SessionState;

/// Renders the handoff outcome and, for a closed wizard (`Finished(0)`),
/// applies the same launch-options reload `/reload` runs: the swap
/// refreshes the profile list, the @-references, and the splash's startup
/// facts, and ONE System block carries the close plus the reload report.
/// A failed reload keeps the old runtime and says so; a non-zero exit or a
/// wizard failure keeps its U05 Error block and never reloads. A lost
/// terminal (`Err`) is returned, never rendered: the caller must stop the
/// loop rather than keep drawing into a terminal it no longer owns.
pub(crate) fn apply_setup_outcome(
    outcome: io::Result<SetupHandoff>,
    app: &mut App,
    state: &mut SessionState,
    options: &GlobalOptions,
) -> io::Result<()> {
    match outcome? {
        SetupHandoff::Finished(0) => {
            let inputs = ReloadInputs::from_launch(options);
            let old = Arc::clone(&app.runtime);
            match reload_runtime(&inputs, &old, state) {
                ReloadOutcome::Reloaded { new, text } => {
                    apply_reloaded_runtime(app, state, new);
                    app.transcript
                        .push(BlockKind::System, format!("Guided setup closed. {text}"));
                }
                ReloadOutcome::Failed(text) => app.transcript.push(
                    BlockKind::Error,
                    format!(
                        "Guided setup closed, but the new configuration could not \
                         be applied: {text}"
                    ),
                ),
            }
        }
        SetupHandoff::Finished(code) => app.transcript.push(
            BlockKind::Error,
            format!(
                "Guided setup ended with exit code {code}. \
                 Nothing was applied that the wizard did not report."
            ),
        ),
        SetupHandoff::Failed(message) => app
            .transcript
            .push(BlockKind::Error, format!("Guided setup failed: {message}")),
    }
    Ok(())
}
