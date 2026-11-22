//! `/setup` in the new surface: the terminal handed to today's guided wizard.
//!
//! The boundary itself is the old surface's and is reused, never copied
//! (`interactive::tui::terminal_handoff`): it releases the alternate screen,
//! runs the wizard in the normal one, and always comes back — on a close, on a
//! cancel, and on a panic inside the wizard. What this module adds is the flow
//! the old surface built (the same `FlowOptions::real` over the launch's own
//! options) and the meaning of the wizard's close: a completed setup is applied
//! by the same reload `/reload` runs, and every other ending keeps the
//! configuration the surface already had.

use std::io;

use super::ops::Asked;
use crate::cli::GlobalOptions;
use crate::interactive::tui::{SetupHandoff, TerminalModes, run_guided_setup};
use crate::setup::flow_options::FlowOptions;

/// What `/setup` says when the run is still working: the wizard takes the whole
/// screen, and a streaming turn behind it would be drawn under the wizard.
pub(crate) const WHILE_BUSY: &str =
    "the run is still working: cancel it before the guided setup takes the terminal";

/// Runs the guided setup over the real stdin/stdout with the terminal released,
/// built from the launch's options exactly as the old surface built it.
pub(crate) fn run(
    modes: &mut dyn TerminalModes,
    options: &GlobalOptions,
) -> io::Result<SetupHandoff> {
    let mut flow = FlowOptions::real(options.clone());
    run_guided_setup(modes, &mut flow)
}

/// What the wizard's close means, in the surface's words. `reload` is the one
/// reload `/reload` runs, called only for a completed setup — exit 0, which is
/// also the wizard's own cancelled-nothing-changed, and therefore reloads into
/// the unchanged line rather than skipping the apply. A lost terminal is
/// returned, never said: the caller must stop drawing into it.
pub(crate) fn landed(
    outcome: io::Result<SetupHandoff>,
    reload: impl FnOnce() -> Asked,
) -> io::Result<Asked> {
    Ok(match outcome? {
        SetupHandoff::Finished(0) => match reload() {
            Asked::Notice(text) => Asked::Notice(format!("Guided setup closed. {text}")),
            Asked::Error(text) => Asked::Error(format!(
                "Guided setup closed, but the new configuration could not be applied: {text}"
            )),
            // `reload` is the reload's own apply and answers with words only.
            other => other,
        },
        SetupHandoff::Finished(code) => Asked::Error(format!(
            "Guided setup ended with exit code {code}. \
             Nothing was applied that the wizard did not report."
        )),
        SetupHandoff::Failed(message) => Asked::Error(format!("Guided setup failed: {message}")),
    })
}
