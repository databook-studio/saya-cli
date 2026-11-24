//! The terminal the runtime paints on, and the way back out of it.
//!
//! A [`Screen`] is the whole of the surface's terminal edge: entering the mode
//! it draws in, painting one frame, and undoing the entry. The real one is
//! today's [`TerminalGuard`], so the runtime and the TUI it replaces put the
//! terminal into raw mode and the alternate screen — and take it out — the same
//! way, through the same code. A test drives a screen of its own, which is why
//! nothing below the trait names a `Terminal<CrosstermBackend<Stdout>>`.

use std::io;
use std::io::Write;
use std::sync::Arc;

use super::model::Model;
use super::run_draw::{self, Facts};
use crate::interactive::tui::{TerminalGuard, TerminalModes, restore_terminal};

/// What a notification writes: the terminal's bell, then the desktop notification
/// sequence terminals that support it read (and others ignore). One write, so a
/// terminal never rings twice for one event.
pub(crate) const ATTENTION: &[u8] = b"\x07\x1b]9;Saya needs attention\x07";

/// Writes [`ATTENTION`] to `out`, outside any frame paint.
pub(crate) fn write_attention(out: &mut impl Write) -> io::Result<()> {
    out.write_all(ATTENTION)?;
    out.flush()
}

/// The terminal the runtime is drawn on.
pub(crate) trait Screen: Send {
    /// Enters the state the surface is drawn in: raw mode, the alternate
    /// screen, bracketed paste.
    fn enter(&mut self) -> io::Result<()>;

    /// Leaves it again. Called on the way out, on an error, and by the panic
    /// hook — so it must be safe to call twice.
    fn restore(&mut self) -> io::Result<()>;

    /// Paints one whole frame.
    fn draw(&mut self, model: &Model, facts: &Facts) -> io::Result<()>;

    /// Rings the terminal's bell once. A screen with no terminal of its own —
    /// a test's — records it and nothing else.
    fn bell(&mut self) -> io::Result<()> {
        Ok(())
    }

    /// Tells the operator the surface needs them: the terminal's bell and the
    /// desktop notification sequence, written outside any frame.
    fn notify(&mut self) -> io::Result<()> {
        Ok(())
    }

    /// The restore a panic hook can call. A hook cannot borrow the runtime, so
    /// a screen that can be put back from anywhere says how; one that cannot
    /// leaves the process's hook alone.
    fn restorer(&self) -> Option<Arc<dyn Fn() + Send + Sync>> {
        None
    }

    /// The terminal's mode pair, so plain line I/O can run in the normal screen
    /// for a while and the surface can take the screen back afterwards (`/setup`
    /// hands the terminal to the guided wizard). `Send`, because the wizard
    /// runs on a thread of its own: it builds its own runtime for the probes,
    /// which it cannot do on this one. `None` on a screen with no terminal of
    /// its own — a test's — which hands nothing over.
    fn modes(&mut self) -> Option<&mut (dyn TerminalModes + Send)> {
        None
    }
}

/// The real terminal: raw mode, the alternate screen and the restore all
/// through today's guard.
pub(crate) struct TtyScreen {
    guard: Option<TerminalGuard>,
}

impl TtyScreen {
    /// A screen that has not entered the terminal yet: [`Screen::enter`] does.
    pub(crate) fn new() -> Self {
        Self { guard: None }
    }
}

impl Screen for TtyScreen {
    fn enter(&mut self) -> io::Result<()> {
        match TerminalGuard::new() {
            Ok(guard) => {
                self.guard = Some(guard);
                Ok(())
            }
            // The guard enables raw mode before it can fail on the alternate
            // screen: a half-entered terminal is repaired before the error is
            // handed back.
            Err(error) => {
                let _ = restore_terminal();
                Err(error)
            }
        }
    }

    fn restore(&mut self) -> io::Result<()> {
        match &mut self.guard {
            Some(guard) => guard.restore_modes(),
            None => Ok(()),
        }
    }

    fn draw(&mut self, model: &Model, facts: &Facts) -> io::Result<()> {
        let Some(guard) = &mut self.guard else {
            return Ok(());
        };
        guard
            .terminal
            .draw(|frame| run_draw::frame(model, facts, frame))?;
        Ok(())
    }

    /// One bell on the real terminal: the byte a terminal rings from, written
    /// where the surface draws rather than through the session.
    fn bell(&mut self) -> io::Result<()> {
        if self.guard.is_none() {
            return Ok(());
        }
        let mut stdout = io::stdout();
        stdout.write_all(b"\x07")?;
        stdout.flush()
    }

    /// The bell and the desktop notification, written after the frame rather
    /// than into it: a terminal that does not know OSC 9 ignores the sequence.
    fn notify(&mut self) -> io::Result<()> {
        if self.guard.is_none() {
            return Ok(());
        }
        write_attention(&mut io::stdout())
    }

    fn restorer(&self) -> Option<Arc<dyn Fn() + Send + Sync>> {
        Some(Arc::new(|| {
            let _ = restore_terminal();
        }))
    }

    fn modes(&mut self) -> Option<&mut (dyn TerminalModes + Send)> {
        // The guard is the old surface's own: its `release`/`reacquire` are the
        // suspendable pair the handoff boundary was written against.
        self.guard
            .as_mut()
            .map(|guard| guard as &mut (dyn TerminalModes + Send))
    }
}
