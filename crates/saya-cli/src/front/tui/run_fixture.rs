//! Test support for the runtime: a screen that records what it was asked to do,
//! a clock a test turns by hand, and a port that records the commands.
//!
//! The whole point of the runtime's seams is that a full loop — queue, update,
//! commands, frames — can be driven without a terminal and without waiting, so
//! what a test needs is exactly these three: somewhere the frames land,
//! something that says what time it is, and something that holds the commands
//! the session would have performed.

use std::io;
use std::sync::Arc;
use std::sync::{Mutex, MutexGuard, PoisonError};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::model::Model;
use super::msg::Msg;
use super::run_draw::{self, Facts};
use super::run_screen::Screen;

/// One thing a fake screen was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Act {
    /// Raw mode and the alternate screen entered.
    Enter,
    /// The way back out taken: raw mode off, the alternate screen left.
    Restore,
    /// One frame painted.
    Draw,
    /// The terminal's bell rung once.
    Bell,
}

/// What a fake screen recorded.
#[derive(Debug, Default)]
pub(crate) struct Recorded {
    /// Every act, in the order they happened.
    pub(crate) acts: Vec<Act>,
    /// The text of every frame painted, in order.
    pub(crate) frames: Vec<String>,
}

impl Recorded {
    /// How many frames were painted.
    pub(crate) fn draws(&self) -> usize {
        self.frames.len()
    }

    /// The frames painted after `act`, in order.
    pub(crate) fn after(&self, act: Act) -> Vec<Act> {
        let at = self.acts.iter().position(|seen| *seen == act);
        match at {
            Some(at) => self.acts[at + 1..].to_vec(),
            None => Vec::new(),
        }
    }

    /// The last frame painted, as the terminal read.
    pub(crate) fn last_frame(&self) -> &str {
        self.frames.last().map_or("", String::as_str)
    }

    /// Whether the terminal was put back at any point.
    pub(crate) fn restored(&self) -> bool {
        self.acts.contains(&Act::Restore)
    }
}

/// A screen over ratatui's test backend: the frame is painted as really as it
/// is on a terminal, and read back out of the buffer.
pub(crate) struct FakeScreen {
    terminal: Terminal<TestBackend>,
    recorded: Arc<Mutex<Recorded>>,
}

impl FakeScreen {
    /// A `width × height` screen.
    pub(crate) fn new(width: u16, height: u16) -> Self {
        Self {
            terminal: Terminal::new(TestBackend::new(width, height))
                .expect("the test backend is built"),
            recorded: Arc::new(Mutex::new(Recorded::default())),
        }
    }

    /// The record, shared with the screen: a test reads it after the runtime has
    /// been moved into a task.
    pub(crate) fn recorded(&self) -> Arc<Mutex<Recorded>> {
        Arc::clone(&self.recorded)
    }

    fn record(&self) -> MutexGuard<'_, Recorded> {
        self.recorded.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Screen for FakeScreen {
    fn enter(&mut self) -> io::Result<()> {
        self.record().acts.push(Act::Enter);
        Ok(())
    }

    fn restore(&mut self) -> io::Result<()> {
        self.record().acts.push(Act::Restore);
        Ok(())
    }

    fn draw(&mut self, model: &Model, facts: &Facts) -> io::Result<()> {
        self.terminal
            .draw(|frame| run_draw::frame(model, facts, frame))
            .expect("the test backend cannot fail to paint");
        let painted = format!("{}", self.terminal.backend());
        let mut recorded = self.record();
        recorded.acts.push(Act::Draw);
        recorded.frames.push(painted);
        Ok(())
    }

    fn bell(&mut self) -> io::Result<()> {
        self.record().acts.push(Act::Bell);
        Ok(())
    }

    fn restorer(&self) -> Option<Arc<dyn Fn() + Send + Sync>> {
        let recorded = Arc::clone(&self.recorded);
        Some(Arc::new(move || {
            recorded
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .acts
                .push(Act::Restore);
        }))
    }
}

/// A model with the composer's text already typed.
pub(crate) fn typed(text: &str) -> Model {
    let mut model = Model::default();
    model.insert(text);
    model
}

/// A model with a turn in flight.
pub(crate) fn busy() -> Model {
    let mut model = Model::default();
    model.start_turn();
    model
}

/// A model saying something in its own voice, with the toast still living.
pub(crate) fn toasting() -> Model {
    let mut model = Model::default();
    model.say("a refused key");
    model
}

/// A key press, as the input thread would read it.
pub(crate) fn key(code: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(code, KeyModifiers::NONE))
}
