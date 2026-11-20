//! How one turn of an open run ended, as an attended surface would show it.
//!
//! A turn can stop on its own question budget, on a progress guard, or on any
//! other failure, and the three read differently: the first is chat's partial
//! wording, the second is a pause the operator continues with another message,
//! the third is a failure. The reading is keyed on the failure CLASS — never on
//! "the turn stopped" — because a backstop is a failure, not a pause.

use saya_agent::BudgetDimension;
use saya_types::CodingRunFailure;

use crate::budget_wording::budget_partial_message;

/// What ended one turn, and the one line that says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TurnStop {
    /// The turn's own question budget stopped it before it answered in full:
    /// chat's partial wording, and the run keeps taking turns.
    Budget {
        dimension: BudgetDimension,
        message: String,
    },
    /// A progress guard stopped the turn. It is a pause, and the message is the
    /// guard's own, naming the key to raise.
    Paused { message: String },
    /// Anything else stopped the turn: a failure, with its own message.
    Failed { message: String },
}

impl TurnStop {
    /// The reading of a turn that stopped on its own question budget.
    pub(crate) fn budget(dimension: BudgetDimension) -> Self {
        Self::Budget {
            dimension,
            message: budget_partial_message(dimension),
        }
    }

    /// The reading of a turn whose latched stop is `failure`.
    ///
    /// A guard class is a pause with the guard's message; every other class —
    /// a backstop included — is a failure, because a backstop is a runaway
    /// bound the operator raised, never an invitation to continue.
    pub(crate) fn of_failure(failure: &CodingRunFailure) -> Self {
        let message = failure.message.as_deref().map_or_else(
            || format!("stopped: the run failed ({})", failure.class.as_str()),
            str::to_owned,
        );
        if super::failure::is_guard(failure.class) {
            Self::Paused { message }
        } else {
            Self::Failed { message }
        }
    }

    /// Whether this stop is a pause a further message continues.
    #[allow(dead_code)] // the attached surface of phase 4.6 reads it; the turn tests read it today
    pub(crate) const fn paused(&self) -> bool {
        matches!(self, Self::Paused { .. })
    }

    /// The one line that says what ended the turn.
    #[allow(dead_code)] // the attached surface of phase 4.6 reads it; the turn tests read it today
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Budget { message, .. } | Self::Paused { message } | Self::Failed { message } => {
                message
            }
        }
    }

    /// The dimension the question budget stopped on, when that is what stopped
    /// the turn.
    #[allow(dead_code)] // the attached surface of phase 4.6 reads it; the turn tests read it today
    pub(crate) const fn dimension(&self) -> Option<BudgetDimension> {
        match self {
            Self::Budget { dimension, .. } => Some(*dimension),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "turn_stop_tests.rs"]
mod tests;
