//! The one non-TUI owner of interactive approval input.
//!
//! TUI keyboard input owns crossterm while raw mode is active. This module
//! refuses that state instead of competing for its process-global reader.

use ratatui::crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, is_raw_mode_enabled},
};
use saya_agent::CancellationToken;
use std::io::{self, IsTerminal, Write};
use std::time::Duration;

const INPUT_LIMIT: usize = 256;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) async fn approval_line(token: &CancellationToken) -> io::Result<Option<String>> {
    if token.is_cancelled() || !io::stdin().is_terminal() {
        return Ok(None);
    }
    let _raw = RawMode::enter()?;
    let mut line = String::new();
    loop {
        if token.is_cancelled() {
            return Ok(None);
        }
        if !event::poll(Duration::ZERO)? {
            tokio::time::sleep(POLL_INTERVAL).await;
            continue;
        }
        // crossterm guarantees that a successful `poll` makes this immediate
        // `read` non-blocking. This owner never enables bracketed paste, so
        // normal pasted bytes arrive as keys and hit `INPUT_LIMIT` before
        // entering `line`.
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Enter => {
                if token.is_cancelled() {
                    return Ok(None);
                }
                eprintln!();
                return Ok(Some(line));
            }
            KeyCode::Esc | KeyCode::Char('c')
                if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
            {
                token.cancel();
                return Ok(None);
            }
            KeyCode::Backspace => {
                if line.pop().is_some() {
                    eprint!("\x08 \x08");
                    let _ = io::stderr().flush();
                }
            }
            KeyCode::Char(character) => {
                let bytes = character.len_utf8();
                if line.len().saturating_add(bytes) > INPUT_LIMIT {
                    return Err(io::Error::other("approval input exceeds 256 bytes"));
                }
                line.push(character);
                eprint!("{character}");
                let _ = io::stderr().flush();
            }
            _ => {}
        }
    }
}

struct RawMode {
    changed: bool,
}

impl RawMode {
    fn enter() -> io::Result<Self> {
        if is_raw_mode_enabled()? {
            return Err(io::Error::other("terminal input is already owned"));
        }
        enable_raw_mode()?;
        Ok(Self { changed: true })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if self.changed {
            let _ = disable_raw_mode();
        }
    }
}
