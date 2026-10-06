//! The U04 handoff boundary between the full-screen TUI and plain line I/O.
//! A future caller (U06) suspends the TUI, runs the existing guided setup in
//! the normal screen, and always comes back to the idle TUI — on success, on
//! a wizard error, on cancellation, and on a panic inside the wizard.

use std::io::{self, BufRead, Write};
use std::panic::{self, AssertUnwindSafe};
use std::time::Duration;

use ratatui::crossterm::{
    event::{EnableBracketedPaste, EnableMouseCapture, poll, read},
    execute,
    terminal::{EnterAlternateScreen, enable_raw_mode},
};

use super::terminal::TerminalGuard;
use crate::setup::flow::run_with;
use crate::setup::flow_options::FlowOptions;

/// The TUI's terminal-mode pair, suspendable so foreign line I/O can run in
/// the normal screen. `release` must undo exactly what the TUI guard set up;
/// `reacquire` must put it back and drain input typed in between.
pub(crate) trait TerminalModes {
    fn release(&mut self) -> io::Result<()>;
    fn reacquire(&mut self) -> io::Result<()>;
}

impl TerminalModes for TerminalGuard {
    fn release(&mut self) -> io::Result<()> {
        self.restore_modes()
    }

    fn reacquire(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        execute!(
            self.terminal.backend_mut(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture
        )?;
        // The screen content was replaced, so ratatui must repaint every
        // cell: clear forces a full redraw on the next draw.
        self.terminal.clear()?;
        // Keystrokes typed while the wizard owned the terminal must never
        // reach the TUI: drain everything already queued before the first
        // repaint.
        while poll(Duration::ZERO)? {
            let _ = read()?;
        }
        Ok(())
    }
}

/// Runs `work` with the TUI's terminal modes released, and ALWAYS reacquires
/// them afterwards — on success, on error, and on panic (the panic is
/// resumed once the terminal is back). A failed release runs no work; the
/// half-released terminal is repaired best-effort first. A failed reacquire
/// is the returned error: the caller must treat the terminal as lost.
pub(crate) fn with_terminal_released<M: TerminalModes, R>(
    modes: &mut M,
    work: impl FnOnce() -> R,
) -> io::Result<R> {
    if let Err(error) = modes.release() {
        let _ = modes.reacquire();
        return Err(error);
    }
    let result = panic::catch_unwind(AssertUnwindSafe(work));
    modes.reacquire().map_err(|error| {
        io::Error::other(format!(
            "the terminal could not be re-entered after the setup handoff: {error}"
        ))
    })?;
    match result {
        Ok(value) => Ok(value),
        Err(payload) => panic::resume_unwind(payload),
    }
}

/// What the guided setup produced, one level above the terminal details.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SetupHandoff {
    /// The wizard's exit code; 0 includes its own cancelled-nothing-changed.
    Finished(i32),
    /// The wizard failed; the message is its rendered error text.
    Failed(String),
}

/// Runs the existing guided setup over the real stdin/stdout in the normal
/// screen: release, wizard, reacquire — always in that order. Prints nothing
/// afterwards; the TUI repaints.
pub(crate) fn run_guided_setup<M: TerminalModes>(
    modes: &mut M,
    options: &mut FlowOptions,
) -> io::Result<SetupHandoff> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    run_guided_setup_with(modes, options, &mut input, &mut output)
}

/// The same handoff over injected streams, so the whole path is testable
/// with scripted input and a captured output.
pub(crate) fn run_guided_setup_with<M: TerminalModes>(
    modes: &mut M,
    options: &mut FlowOptions,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> io::Result<SetupHandoff> {
    with_terminal_released(modes, || match run_with(options, input, output) {
        Ok(code) => SetupHandoff::Finished(code),
        Err(error) => SetupHandoff::Failed(error.to_string()),
    })
}
