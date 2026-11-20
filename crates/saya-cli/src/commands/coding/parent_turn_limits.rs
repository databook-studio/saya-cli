//! One open run's per-turn inputs and readings.
//!
//! A surface states the limits its turns run under before a turn, and reads how
//! that turn ended after it. Both belong to the run — it outlives its turns —
//! and the run takes one turn at a time, so the two slots never describe
//! different turns.

use saya_agent::TurnPolicy;
use std::sync::Mutex;

use super::TurnStop;

/// The turn policy a surface set, and how the turn that just ended ended.
#[derive(Debug, Default)]
pub(super) struct TurnInputs {
    policy: Mutex<Option<TurnPolicy>>,
    stop: Mutex<Option<TurnStop>>,
}

impl TurnInputs {
    /// Sets the limits every turn applies, or clears them. `None` — the default
    /// — is a turn with no turn-scoped budget.
    pub(super) fn set_policy(&self, policy: Option<TurnPolicy>) {
        *self
            .policy
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = policy;
    }

    /// The limits the next turn applies.
    pub(super) fn policy(&self) -> Option<TurnPolicy> {
        *self
            .policy
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// States how the turn that just ended ended, for the surface that ran it.
    pub(super) fn note_stop(&self, stop: Option<TurnStop>) {
        *self
            .stop
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = stop;
    }

    /// How the turn that ended most recently ended.
    pub(super) fn stop(&self) -> Option<TurnStop> {
        self.stop
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}
