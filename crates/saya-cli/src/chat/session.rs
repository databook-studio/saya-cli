//! The session state machine: one run, one turn at a time, one waiting message.
//!
//! A session owns the run's life and nothing below it: it never renders, prompts
//! or decides a call; its only edge to the engine is [`RunPort`](super::RunPort).

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::{
    ChatEvent, ChatFront,
    port::{RunPort, TurnOutcome, offer_turn},
    queue::{Enqueued, SendQueue},
};

/// What a displacing send tells the surface: the waiting message never vanishes silently.
const REPLACED: &str =
    "the message waiting to run was replaced by this one; the earlier message was not sent";

/// Why a quit is refused while the run is working.
const CANCEL_FIRST: &str = "the run is still working: cancel it before quitting";

/// Where the session is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionState {
    /// The run is being composed and opened.
    Preparing,
    Ready,
    /// A turn is running.
    Busy,
    /// The run could not be prepared; the operator's next message retries.
    Degraded,
    Closing,
    /// The run is finalized; no message is admitted.
    Closed,
}

/// What became of one message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    /// The message ran as a turn, which has now ended.
    Ran,
    /// A turn, or a prepare, is in flight: the message waits for it.
    Queued,
    /// Closing or closed: the message was not admitted.
    Refused,
}

/// What a send may do, decided under the one lock so two sends cannot both run.
enum Decision {
    Run,
    Hold,
    /// The run failed to prepare: keep the message and prepare again.
    Retry,
    Refused,
}

/// The session's own state, behind one lock.
struct State {
    phase: SessionState,
    queue: SendQueue,
}

/// One chat session.
pub(crate) struct ChatSession {
    port: Arc<dyn RunPort>,
    front: Arc<dyn ChatFront>,
    inner: Mutex<State>,
}

impl ChatSession {
    /// A session that has begun preparing its run.
    pub(crate) fn new(port: Arc<dyn RunPort>, front: Arc<dyn ChatFront>) -> Self {
        Self {
            port,
            front,
            inner: Mutex::new(State {
                phase: SessionState::Preparing,
                queue: SendQueue::new(),
            }),
        }
    }

    pub(crate) fn state(&self) -> SessionState {
        self.lock().phase
    }

    /// Composes and opens the run, then runs anything sent while it prepared:
    /// `Ready`, or `Degraded` with the engine's own words shown.
    pub(crate) async fn open(&self) {
        if !self.prepare().await {
            return;
        }
        if let Some(message) = self.waiting() {
            self.run_turns(message).await;
        }
    }

    pub(crate) async fn send(&self, message: String) -> Admission {
        match self.decide() {
            Decision::Run => {
                self.run_turns(message).await;
                Admission::Ran
            }
            Decision::Hold => {
                self.hold(message);
                Admission::Queued
            }
            Decision::Retry => {
                // The message that follows a failed prepare is the operator's
                // own words about what to do next: it waits while the run is
                // prepared again, and runs on the run that opens.
                self.hold(message);
                if !self.prepare().await {
                    return Admission::Queued;
                }
                match self.waiting() {
                    Some(retained) => {
                        self.run_turns(retained).await;
                        Admission::Ran
                    }
                    None => Admission::Queued,
                }
            }
            Decision::Refused => Admission::Refused,
        }
    }

    /// Asks the turn in flight to stop; it ends as `Cancelled` and the session
    /// settles back to `Ready`.
    pub(crate) async fn cancel(&self) {
        self.port.cancel_turn().await;
    }

    /// Closes the run. A turn in flight is not abandoned: the operator cancels it
    /// first, and a quit before that is refused in plain words.
    pub(crate) async fn quit(&self) -> Result<(), String> {
        {
            let mut state = self.lock();
            match state.phase {
                SessionState::Closed | SessionState::Closing => return Ok(()),
                SessionState::Busy | SessionState::Preparing => {
                    return Err(CANCEL_FIRST.to_owned());
                }
                SessionState::Ready | SessionState::Degraded => {
                    state.phase = SessionState::Closing;
                }
            }
        }
        self.port.close().await;
        self.lock().phase = SessionState::Closed;
        Ok(())
    }

    /// Prepares the run and tells the front what it is talking to.
    async fn prepare(&self) -> bool {
        self.lock().phase = SessionState::Preparing;
        match self.port.prepare().await {
            Ok(prepared) => {
                self.lock().phase = SessionState::Ready;
                self.front.emit(ChatEvent::Ready {
                    db: prepared.db,
                    workspace: prepared.workspace,
                });
                true
            }
            Err(message) => {
                self.lock().phase = SessionState::Degraded;
                self.front.emit(ChatEvent::Notice {
                    message: format!("the run could not be prepared: {message}"),
                });
                false
            }
        }
    }

    /// What this send may do now.
    fn decide(&self) -> Decision {
        let mut state = self.lock();
        match state.phase {
            SessionState::Ready => {
                state.phase = SessionState::Busy;
                Decision::Run
            }
            SessionState::Preparing | SessionState::Busy => Decision::Hold,
            SessionState::Degraded => Decision::Retry,
            SessionState::Closing | SessionState::Closed => Decision::Refused,
        }
    }

    /// Holds one message for the turn in flight; one displacing it is reported.
    fn hold(&self, message: String) {
        if self.lock().queue.offer(message) == Enqueued::Replaced {
            self.front.emit(ChatEvent::Notice {
                message: REPLACED.to_owned(),
            });
        }
    }

    fn waiting(&self) -> Option<String> {
        self.lock().queue.take()
    }

    /// Runs one turn and then whatever the queue holds behind it, in the order the
    /// operator sent them, then settles back to `Ready`.
    async fn run_turns(&self, message: String) {
        let mut next = Some(message);
        while let Some(message) = next {
            match offer_turn(self.port.as_ref(), self.front.sink(), message).await {
                TurnOutcome::Ended(outcome) => self.front.emit(ChatEvent::TurnEnded(outcome)),
                TurnOutcome::Refused(message) => self.front.emit(ChatEvent::Notice { message }),
                TurnOutcome::Busy(message) => {
                    // The engine never released its slot. The message keeps its
                    // place instead of being handed back as an error.
                    self.hold(message);
                    return;
                }
            }
            next = self.next_or_ready();
        }
    }

    fn next_or_ready(&self) -> Option<String> {
        let mut state = self.lock();
        match state.queue.take() {
            Some(message) => Some(message),
            None => {
                state.phase = SessionState::Ready;
                None
            }
        }
    }

    /// The state lock. A panic in a turn must not leave the session unusable: the
    /// value behind a poisoned lock is the last one written, which is where the
    /// session was.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
#[path = "session_support.rs"]
mod support;

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "session_busy_tests.rs"]
mod busy_tests;
