//! A settled turn's meters: the share of the model's window its last request
//! used, and what the session has cost so far — as the queue's one message, so
//! the row is a pure function of what the run reported.
//!
//! The turn folds into the session's books here, at the same settlement that
//! takes its record: the spend is a session total, and the last request is the
//! one the window is measured against. Neither figure is inferred. An unknown
//! window, or no reported cost, is absent, and the row says nothing about it.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::run_front::Forward;
use super::view::Meters;
use crate::chat::ChatOutcome;
use crate::front::line::TurnRecord;
use crate::interactive::session::state::SessionState;

impl Forward {
    /// Hands the front the session's books and the window its model is known
    /// to have: what a settled turn folds into, and what the meter divides by.
    pub(crate) fn watch(&self, state: Arc<Mutex<SessionState>>, window: Option<u64>) {
        *self.book() = Some(state);
        self.set_window(window);
    }

    /// States the window of the model now in force: a `/model` that changed it
    /// changes what the meter divides by, without touching the spend.
    pub(crate) fn set_window(&self, window: Option<u64>) {
        *self.window.lock().unwrap_or_else(PoisonError::into_inner) = window;
    }

    /// The session's books, when the launch handed them over.
    pub(super) fn book(&self) -> MutexGuard<'_, Option<Arc<Mutex<SessionState>>>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Folds the turn into the session's books and hands the row its meters.
pub(super) fn note(front: &Forward, outcome: &ChatOutcome, record: &TurnRecord) {
    let books = front.book().clone();
    let (input, cost) = match books {
        Some(books) => {
            let mut state = books
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            record.totals.fold_into(&mut state.usage, outcome);
            let sent = record.totals.latest.map(|usage| usage.input_tokens);
            state.usage.note_request(sent, record.totals.cost_micros);
            (state.usage.last_input_tokens(), state.usage.cost_micros())
        }
        // No books were handed over — a front built without a session — so the
        // turn's own report is all there is to state.
        None => (
            record.totals.latest.map(|usage| usage.input_tokens),
            record.totals.cost_micros,
        ),
    };
    let window = *front
        .window
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let meters = Meters {
        input_tokens: input,
        window_tokens: window,
        cost_micros: cost,
    };
    if meters != Meters::default() {
        let _ = front.msgs.send(super::msg::Msg::Meters(meters));
    }
}
