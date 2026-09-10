//! The lifecycle transition records: what the engine asks the sink to
//! record, and what each one becomes — a state-machine event, a journal
//! event, and a store status. The payload-free variants map to
//! [`RunTransition`](super::state::RunTransition) one to one; `Pause` and
//! `Fail` carry the reason and typed code the state machine deliberately
//! leaves out.

use saya_store::RunStatus;
use saya_types::{PauseReason, RunEvent, RunFailureCode};

use super::state::RunTransition;

/// A lifecycle transition the engine asks the sink to record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransitionEvent {
    Approve,
    Begin,
    Resume,
    Complete,
    Cancel,
    Pause(PauseReason),
    Fail(RunFailureCode),
}

impl TransitionEvent {
    /// What this event becomes: the state-machine event, the journal event
    /// (`None` for `Begin`/`Resume` — bare `Executing` has no journal event;
    /// the steps' `StepStarted` events tell that story), and the store
    /// status with its failure code.
    pub(super) fn record(
        self,
    ) -> (
        RunTransition,
        Option<RunEvent>,
        RunStatus,
        Option<RunFailureCode>,
    ) {
        match self {
            Self::Approve => (
                RunTransition::Approve,
                Some(RunEvent::PlanApproved),
                RunStatus::Approved,
                None,
            ),
            Self::Begin => (RunTransition::Begin, None, RunStatus::Executing, None),
            Self::Resume => (RunTransition::Resume, None, RunStatus::Executing, None),
            Self::Complete => (
                RunTransition::Complete,
                Some(RunEvent::Completed),
                RunStatus::Completed,
                None,
            ),
            Self::Cancel => (
                RunTransition::Cancel,
                Some(RunEvent::Cancelled),
                RunStatus::Cancelled,
                None,
            ),
            Self::Pause(reason) => (
                RunTransition::Pause,
                Some(RunEvent::Paused { reason }),
                RunStatus::Paused,
                None,
            ),
            Self::Fail(code) => (
                RunTransition::Fail,
                Some(RunEvent::Failed { code }),
                RunStatus::Failed,
                Some(code),
            ),
        }
    }
}
