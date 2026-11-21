//! Terminal setup and teardown for the full-screen TUI.

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::{
    cursor::Show,
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Stdout};

/// RAII guard: enters raw mode + the alternate screen on construction and
/// always restores the terminal on drop, including on panic/early return.
pub(crate) struct TerminalGuard {
    pub(crate) terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    pub(crate) fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture
        )?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self { terminal })
    }

    /// The one restore sequence: raw mode off, mouse capture, bracketed
    /// paste, and the alternate screen undone, cursor shown. `Drop`, the
    /// handoff's `release` and the runtime's panic hook share
    /// [`restore_terminal`], so the teardown paths cannot drift.
    pub(crate) fn restore_modes(&mut self) -> io::Result<()> {
        restore_terminal()
    }
}

/// The restore sequence, for the callers that own no guard: `Drop`, the setup
/// handoff's `release`, and the runtime's panic hook — which cannot borrow a
/// terminal at all, because it runs while the loop is unwinding.
///
/// Raw mode off, mouse capture and bracketed paste undone, the alternate screen
/// left, the cursor shown. It says exactly what [`TerminalGuard::new`] set up, in
/// the order it set it up.
pub(crate) fn restore_terminal() -> io::Result<()> {
    let raw = disable_raw_mode();
    let screen = execute!(
        io::stdout(),
        DisableMouseCapture,
        DisableBracketedPaste,
        LeaveAlternateScreen,
        Show
    );
    raw.and(screen)
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore_modes();
    }
}
