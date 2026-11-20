//! The database plugin's turn policy: the chat's question budget, stated as the
//! limits the core enforces turn by turn.
//!
//! The numbers stay where an operator already configures them —
//! `[limits.session]`, the table chat's question budget has always read — so
//! moving the budget onto the core changes which engine counts, not what an
//! operator sets. A job surface asks for no policy at all: a job's turns are
//! bounded by its own progress guards and `[limits.job]` backstops.

use saya_agent::{AnsweringLimits, RunContext, RunSurface, TurnPolicy};
use std::time::Duration;

use super::{DatabasePlugin, Surface};
use crate::agent::limits::{AnsweringSurface, answering_deadline, answering_limits};
use crate::config::runtime::RuntimeConfig;

/// One chat's `[limits.session]` as the budget of a single turn: the three
/// admission ceilings plus the elapsed deadline the surface arms itself.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SessionBudget {
    answering: AnsweringLimits,
    elapsed: Option<Duration>,
}

impl SessionBudget {
    /// The session row of a resolved config — the table chat's question budget
    /// already lives in.
    pub(crate) fn of(runtime: &RuntimeConfig) -> Self {
        Self {
            answering: answering_limits(runtime, AnsweringSurface::Session),
            elapsed: answering_deadline(runtime, AnsweringSurface::Session),
        }
    }

    /// The same four dimensions as a turn policy: the plugin's whole ask.
    fn policy(self) -> TurnPolicy {
        TurnPolicy::from_answering(self.answering, self.elapsed)
    }
}

impl DatabasePlugin {
    /// The turn limits this plugin asks the core for, or `None` to ask for
    /// none.
    ///
    /// Only the chat's own turn is a database question: it is a turn of a
    /// conversation with a database configured. A headless, background or
    /// helper run keeps its own bounds, so the plugin asks for nothing there
    /// even when the run has a database.
    pub(super) fn turn_policy_for(&self, ctx: &RunContext) -> Option<TurnPolicy> {
        match (&self.surface, ctx.surface) {
            (Surface::Chat(chat), RunSurface::Chat) => Some(chat.session.policy()),
            _ => None,
        }
    }
}
