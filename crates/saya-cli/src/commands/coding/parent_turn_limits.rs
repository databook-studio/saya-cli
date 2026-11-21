//! One open run's per-turn inputs and readings.
//!
//! A surface states the inputs its turns run under when it opens the run — the
//! turn policy, and the plugins the run may activate — and reads how a turn
//! ended after it. Both belong to the run — it outlives its turns — and the run
//! takes one turn at a time, so the two slots never describe different turns.

use saya_agent::{CancellationToken, PluginRegistry, RunContext, TurnPolicy};
use std::sync::Mutex;

use crate::commands::coding::TurnStop;

/// The per-turn inputs a surface states when it opens a run. They are given at
/// open, not set afterwards: `run_open` runs the run's first turn, so a policy
/// a caller could only state later would never bound it.
#[derive(Default)]
pub(crate) struct TurnInputs {
    /// The limits the first turn — and every turn after it, until a surface
    /// states others — runs under.
    pub(crate) policy: Option<TurnPolicy>,
    /// The plugins this run may activate, or `None` — every job surface today —
    /// for a run that consults no registry.
    pub(crate) plugins: Option<RunPlugins>,
}

/// The plugin registry a surface supplies at open, and the context its plugins
/// activate for.
pub(crate) struct RunPlugins {
    pub(crate) registry: PluginRegistry,
    pub(crate) context: RunContext,
}

/// The run's live turn state: the policy a surface set, how the turn that just
/// ended ended, and the stop signal of the turn running now.
#[derive(Default)]
pub(crate) struct TurnState {
    policy: Mutex<Option<TurnPolicy>>,
    stop: Mutex<Option<TurnStop>>,
    turn: Mutex<Option<CancellationToken>>,
}

impl TurnState {
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

    /// Starts a turn on `run`: the turn's stop signal is a child of the run's,
    /// so a stop of the run ends the turn and a stop of the turn leaves the run
    /// running.
    pub(crate) fn begin_turn(&self, run: &CancellationToken) -> TurnToken<'_> {
        let token = run.child();
        *self
            .turn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(token.clone());
        TurnToken { state: self, token }
    }

    /// The stop signal of the turn running now, for the surface that cancels it.
    pub(super) fn turn_cancellation(&self) -> Option<CancellationToken> {
        self.turn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// One turn's stop signal, held for as long as the turn runs: dropping it
/// clears the run's turn slot, so a surface cancels a turn that is running and
/// never the one that ended.
pub(crate) struct TurnToken<'a> {
    state: &'a TurnState,
    token: CancellationToken,
}

impl TurnToken<'_> {
    pub(crate) fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}

impl Drop for TurnToken<'_> {
    fn drop(&mut self) {
        *self
            .state
            .turn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}
