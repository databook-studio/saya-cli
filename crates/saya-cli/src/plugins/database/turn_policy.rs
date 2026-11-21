//! The database plugin's budget: the work one turn's database use may spend.
//!
//! The budget no longer bounds every turn of a chat. It arms on the turn's
//! first database use and never before it (`ref/05`, §2-§3): a turn that never
//! touches a database spends none of it, and a job surface asks for none at all
//! — a job's turns are bounded by its own progress guards and the `[limits.job]`
//! backstops.
//!
//! The numbers stay where an operator already sets them — `[limits.database]`,
//! and the `ask` surface's override of it — so moving the budget onto the
//! plugin changes which engine counts, not what an operator writes.

use saya_agent::{AnsweringLimits, RunContext, RunSurface, TurnPolicy};
use saya_config::ResolvedAnsweringLimits;
use std::time::Duration;

use super::{DatabasePlugin, Surface};
use crate::config::runtime::RuntimeConfig;

/// One resolved answering table as the ceilings a turn's database work is
/// bounded by: the three admission dimensions plus the elapsed deadline the
/// turn's owner arms.
#[derive(Debug, Clone, Copy)]
struct Ceilings {
    answering: AnsweringLimits,
    elapsed: Option<Duration>,
}

impl Ceilings {
    /// The table's four dimensions, an unlimited one carried as absence.
    fn of(limits: ResolvedAnsweringLimits) -> Self {
        Self {
            answering: AnsweringLimits {
                max_requests: limits.requests.value(),
                max_tool_calls: limits.tool_calls.value(),
                max_reported_tokens: limits.known_tokens.value(),
            },
            elapsed: limits.elapsed_seconds.duration(),
        }
    }

    /// The same four dimensions as a turn policy: what the core arms on the
    /// turn's first database use.
    fn policy(self) -> TurnPolicy {
        TurnPolicy::from_answering(self.answering, self.elapsed)
    }
}

/// The budget this plugin's turns spend.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DatabaseBudget {
    ceilings: Ceilings,
}

impl DatabaseBudget {
    /// A chat turn's database budget: `[limits.database]`.
    pub(crate) fn of(runtime: &RuntimeConfig) -> Self {
        Self {
            ceilings: Ceilings::of(runtime.resolved.limits.database),
        }
    }

    /// The `ask` surface's budget: `[limits.ask]` as an override of the database
    /// table, every key it leaves unset already fallen back at resolve time.
    #[allow(dead_code)] // the ask front is its caller (ref/06, unit L3)
    pub(crate) fn ask(runtime: &RuntimeConfig) -> Self {
        Self {
            ceilings: Ceilings::of(runtime.resolved.limits.ask_override),
        }
    }

    /// The four dimensions as the one policy the core arms.
    fn policy(self) -> TurnPolicy {
        self.ceilings.policy()
    }
}

impl DatabasePlugin {
    /// The budget to arm on a turn's first use of this plugin, or `None` for a
    /// surface that spends none.
    ///
    /// Only the chat's own turn is a database question: it is a turn of a
    /// conversation with a database configured, so its first database use arms
    /// `[limits.database]` (or, for the ask surface's plugin, that table's
    /// override). A headless, background or helper run keeps its own bounds, so
    /// the plugin arms nothing there even when the run has a database.
    pub(super) fn policy_on_use_for(&self, ctx: &RunContext) -> Option<TurnPolicy> {
        match (&self.surface, ctx.surface) {
            (Surface::Chat(chat), RunSurface::Chat) => Some(chat.budget.policy()),
            _ => None,
        }
    }
}

#[cfg(all(test, unix))]
#[path = "turn_policy_arming_tests.rs"]
mod arming_tests;
