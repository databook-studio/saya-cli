//! `/setup` from the loop that owns the terminal: the handoff itself.
//!
//! The boundary is the kept leaf (`interactive::tui::terminal_handoff`) and the
//! wizard is the old surface's flow, run by the port; what lives here is the
//! ordering the loop alone can state. The reader is stopped *before* the screen
//! is released and started again only once it is back, so the wizard is the one
//! reading stdin while it owns it; and the wizard runs on a thread of its own,
//! because it builds the runtime its probes run on and cannot build one inside
//! the surface's.

use std::io;
use std::sync::Arc;

use super::run_cmd::CmdPort;
use super::run_input::InputGate;
use super::run_screen::Screen;

/// Hands `screen`'s terminal to the guided wizard and takes it back, with
/// `input` stopped for the whole handoff. A lost terminal is returned: the
/// caller must stop drawing rather than keep drawing into a terminal it no
/// longer owns. A panic inside the wizard is resumed here, once the terminal is
/// back — the boundary reacquires first, then re-raises.
pub(crate) fn handoff<P: CmdPort + ?Sized>(
    screen: &mut impl Screen,
    input: Option<&Arc<InputGate>>,
    port: &P,
) -> io::Result<()> {
    if let Some(input) = input {
        input.pause();
    }
    let outcome = match screen.modes() {
        // The scope joins the wizard before the surface draws again, so the
        // terminal has exactly one owner at a time.
        Some(modes) => {
            match std::thread::scope(|scope| scope.spawn(|| port.guided_setup(modes)).join()) {
                Ok(outcome) => outcome,
                Err(payload) => std::panic::resume_unwind(payload),
            }
        }
        // A screen with no terminal of its own (a test's) has no normal screen
        // to hand over, so there is nothing to run the wizard on.
        None => Ok(()),
    };
    if let Some(input) = input {
        input.resume();
    }
    outcome
}
