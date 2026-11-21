//! One open run's per-turn inputs and readings.
//!
//! A surface states the inputs its turns run under when it opens the run — the
//! turn policy, and the plugins the run may activate — and reads how a turn
//! ended after it. Both belong to the run — it outlives its turns — and the run
//! takes one turn at a time, so the two slots never describe different turns.

use saya_agent::{CancellationReason, CancellationToken, PluginRegistry, RunContext, TurnPolicy};
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
    /// The conversation the run begins with, oldest pair first: a session hands
    /// over the transcript of the run before it when it prepares this one. Empty
    /// — every job surface today — is a run whose first turn carries no history.
    pub(crate) seed: Vec<(String, String)>,
}

/// The plugin registry a surface supplies at open, and the context its plugins
/// activate for.
pub(crate) struct RunPlugins {
    pub(crate) registry: PluginRegistry,
    pub(crate) context: RunContext,
    /// The primary connection a call that names none resolves to, when the
    /// surface resolved one: it is what the database lane's `sql:<primary>`
    /// token names, so a call that names no connection is approved — and its
    /// answer remembered — as the primary database. `None` — a surface with no
    /// database, or one whose primary cannot be resolved — suggests no token,
    /// and such a call keeps asking.
    pub(crate) primary: Option<String>,
}

/// The run's live turn state: the policy a surface set, how the turn that just
/// ended ended, the stop signal of the turn running now, and the reason that
/// signal latched when the turn finished.
#[derive(Default)]
pub(crate) struct TurnState {
    policy: Mutex<Option<TurnPolicy>>,
    stop: Mutex<Option<TurnStop>>,
    turn: Mutex<Option<CancellationToken>>,
    /// Why the turn that ended most recently stopped, kept after its token is
    /// cleared: a budget stop latches there and never on the run, so this is
    /// the only place the finished turn's cause survives for the job surface.
    ended: Mutex<Option<CancellationReason>>,
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
        *self
            .ended
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        TurnToken { state: self, token }
    }

    /// Why the turn running now stopped, when its own token latched a reason.
    pub(super) fn live_stop(&self) -> Option<CancellationReason> {
        self.turn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .and_then(CancellationToken::reason)
    }

    /// Why the turn that ended most recently stopped, or `None` for one that
    /// simply finished. Cleared when the next turn begins.
    pub(super) fn ended_stop(&self) -> Option<CancellationReason> {
        *self
            .ended
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
        // Keep the reason the turn ended on before clearing its slot: a stop
        // latched on the turn's token is the job's cause too.
        *self
            .state
            .ended
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = self.token.reason();
        *self
            .state
            .turn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}
