//! The runtime's other two seams in a test: a clock a test turns by hand, and a
//! port that records the commands instead of performing them.

use std::sync::Arc;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use saya_agent::PermissionMode;

use super::run_clock::Clock;
use super::run_cmd::CmdPort;
use crate::agent::tools::consent::Answer;

/// A clock a test turns by hand: the rate limit is the one thing that reads it,
/// so a test that wants "no frame before 33 ms" never waits for one.
#[derive(Debug, Clone)]
pub(crate) struct FakeClock {
    now: Arc<Mutex<Instant>>,
}

impl FakeClock {
    /// A clock stopped where it was made.
    pub(crate) fn new() -> Self {
        Self {
            now: Arc::new(Mutex::new(Instant::now())),
        }
    }

    /// Moves it forward by `by`.
    pub(crate) fn advance(&self, by: Duration) {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner) += by;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What a fake port was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Call {
    Send(String),
    Cancel,
    Answer(u64, Answer),
    SwitchMode(PermissionMode),
    /// The text handed to the clipboard (`02` §6's `c`).
    Copy(String),
}

/// A port that records instead of performing, and panics on request.
#[derive(Debug, Default)]
pub(crate) struct FakePort {
    calls: Mutex<Vec<Call>>,
    panics: Mutex<bool>,
}

impl FakePort {
    /// A port that records.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A port whose every command panics, as a session's own call can: what the
    /// test watches then is the runtime's handling of the unwind.
    pub(crate) fn panicking() -> Self {
        let port = Self::default();
        *port.panics.lock().unwrap_or_else(PoisonError::into_inner) = true;
        port
    }

    /// The commands performed so far, in order.
    pub(crate) fn calls(&self) -> Vec<Call> {
        self.lock().clone()
    }

    fn record(&self, call: Call) {
        if *self.panics.lock().unwrap_or_else(PoisonError::into_inner) {
            panic!("the port was asked to do something it cannot");
        }
        self.lock().push(call);
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Call>> {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl CmdPort for FakePort {
    fn send(&self, message: String) {
        self.record(Call::Send(message));
    }

    fn cancel(&self) {
        self.record(Call::Cancel);
    }

    fn answer(&self, id: u64, answer: Answer) {
        self.record(Call::Answer(id, answer));
    }

    fn switch_mode(&self, mode: PermissionMode) {
        self.record(Call::SwitchMode(mode));
    }

    fn copy(&self, text: String) {
        self.record(Call::Copy(text));
    }
}
