//! The surface's own last words, and how long they live (`02` §4).
//!
//! A toast is the one thing the surface says without being asked: a refused
//! key, a worker's word. `02` §4 gives it four seconds and the status line's
//! right side while it lives, and it is *ticks* that spend it — the runtime's
//! tick is the only clock the model has, so the value counts the ticks it has
//! left rather than reading a time.
//!
//! T3 defines the tick and its period: `Msg::Tick` is "the 250 ms tick that
//! drives the spinner and expires a toast". That period is named here, once, so
//! the four seconds are sixteen ticks and nothing else has to know why.

use std::time::Duration;

/// The tick the runtime sends: `Msg::Tick`'s 250 ms, as T3's own doc words it.
pub(crate) const TICK: Duration = Duration::from_millis(250);

/// How long a toast lives: `02` §4's four seconds.
pub(crate) const LIFETIME: Duration = Duration::from_secs(4);

/// `LIFETIME` in ticks of `TICK`: sixteen. A toast is spent by counting these.
pub(crate) const TICKS: u32 = (LIFETIME.as_millis() / TICK.as_millis()) as u32;

/// The one thing the surface says in its own voice, and the ticks it has left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Toast {
    /// What is said, word for word.
    text: String,
    /// The ticks left before it expires; zero means it is spent.
    remaining: u32,
}

impl Toast {
    /// A toast saying `text`, with the whole of `02` §4's four seconds ahead of
    /// it. A new toast starts the clock again rather than inheriting the last
    /// one's, because it is the new words the operator has to be able to read.
    pub(crate) fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            remaining: TICKS,
        }
    }

    /// What is said.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// The ticks left before it expires.
    pub(crate) const fn remaining(&self) -> u32 {
        self.remaining
    }

    /// Whether its four seconds are spent.
    pub(crate) const fn expired(&self) -> bool {
        self.remaining == 0
    }

    /// One tick of its life: returns whether it is still to be said.
    ///
    /// A tick is `TICK` of a life that is `TICKS` ticks long, so the sixteenth
    /// tick is the one that spends the last of `02` §4's four seconds.
    pub(crate) fn tick(&mut self) -> bool {
        self.remaining = self.remaining.saturating_sub(1);
        !self.expired()
    }
}
