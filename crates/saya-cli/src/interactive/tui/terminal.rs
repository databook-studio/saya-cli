//! Terminal setup and teardown for the full-screen TUI.

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::{
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
    /// paste, and the alternate screen undone, cursor shown. `Drop` and the
    /// handoff's `release` share it so the two teardown paths cannot drift.
    pub(crate) fn restore_modes(&mut self) -> io::Result<()> {
        let raw = disable_raw_mode();
        let screen = execute!(
            self.terminal.backend_mut(),
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let cursor = self.terminal.show_cursor();
        raw.and(screen).and(cursor)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore_modes();
    }
}
