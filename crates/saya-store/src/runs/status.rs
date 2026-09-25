//! Run and step statuses, with the transition tables that make a status
//! change a state-machine step rather than a free overwrite.
//!
//! The run machine is `planned → approved → executing ⇄ paused → completed |
//! failed | cancelled`: there is no implicit approval, a paused run resumes
//! into executing, and a terminal status never leaves. Steps follow
//! `pending → running → done | failed`, and a failed step may restart — the
//! engine's bounded retry — but a step is never born already finished.

use saya_types::RunFailureCode;

/// The run lifecycle. Re-recording the current status is idempotent, so an
/// engine re-mirroring a transition it already wrote is a no-op, not an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Planned,
    Approved,
    Executing,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Approved => "approved",
            Self::Executing => "executing",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "planned" => Self::Planned,
            "approved" => Self::Approved,
            "executing" => Self::Executing,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    /// True when `from → to` is a legal transition. Cancelling is legal from
    /// every non-terminal state; completing and failing belong to executing;
    /// pausing and resuming belong to the executing ⇄ paused pair.
    pub(crate) fn can_transition(from: Self, to: Self) -> bool {
        if from == to {
            return true;
        }
        matches!(
            (from, to),
            (Self::Planned, Self::Approved)
                | (Self::Planned, Self::Cancelled)
                | (Self::Approved, Self::Executing)
                | (Self::Approved, Self::Cancelled)
                | (Self::Executing, Self::Paused)
                | (Self::Executing, Self::Completed)
                | (Self::Executing, Self::Failed)
                | (Self::Executing, Self::Cancelled)
                | (Self::Paused, Self::Executing)
                | (Self::Paused, Self::Cancelled)
        )
    }
}

/// One step's lifecycle. `failed → running` is the engine's bounded step
/// retry; every other exit from `running` is final for the step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStepStatus {
    Pending,
    Running,
    Done,
    Failed,
}

impl RunStepStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "running" => Self::Running,
            "done" => Self::Done,
            "failed" => Self::Failed,
            _ => return None,
        })
    }

    /// A step's first sighting: it exists because the plan bound it (pending)
    /// or because its episode began (running) — never already finished.
    pub(crate) fn is_initial(self) -> bool {
        !matches!(self, Self::Done | Self::Failed)
    }

    pub(crate) fn can_transition(from: Self, to: Self) -> bool {
        if from == to {
            return true;
        }
        matches!(
            (from, to),
            (Self::Pending, Self::Running)
                | (Self::Running, Self::Done)
                | (Self::Running, Self::Failed)
                | (Self::Failed, Self::Running)
        )
    }
}

/// The wire form of a typed failure code, mirroring `RunFailureCode`'s serde
/// names. The type is `#[non_exhaustive]`, so a code this build does not know
/// has no wire form and is refused rather than stored under a guess.
pub(crate) fn failure_code_str(code: RunFailureCode) -> Option<&'static str> {
    match code {
        RunFailureCode::SafetyQuery => Some("safety_query"),
        RunFailureCode::Provider => Some("provider"),
        RunFailureCode::ConnectionConfig => Some("connection_config"),
        _ => None,
    }
}

pub(crate) fn parse_failure_code(value: &str) -> Option<RunFailureCode> {
    match value {
        "safety_query" => Some(RunFailureCode::SafetyQuery),
        "provider" => Some(RunFailureCode::Provider),
        "connection_config" => Some(RunFailureCode::ConnectionConfig),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{RunStatus, RunStepStatus};

    #[test]
    fn statuses_round_trip() {
        for status in [
            RunStatus::Planned,
            RunStatus::Approved,
            RunStatus::Executing,
            RunStatus::Paused,
            RunStatus::Completed,
            RunStatus::Failed,
            RunStatus::Cancelled,
        ] {
            assert_eq!(RunStatus::parse(status.as_str()), Some(status));
        }
        for status in [
            RunStepStatus::Pending,
            RunStepStatus::Running,
            RunStepStatus::Done,
            RunStepStatus::Failed,
        ] {
            assert_eq!(RunStepStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(RunStatus::parse("bogus"), None);
        assert_eq!(RunStepStatus::parse("bogus"), None);
    }

    #[test]
    fn the_run_state_machine_is_exact() {
        let legal = [
            (RunStatus::Planned, RunStatus::Approved),
            (RunStatus::Planned, RunStatus::Cancelled),
            (RunStatus::Approved, RunStatus::Executing),
            (RunStatus::Approved, RunStatus::Cancelled),
            (RunStatus::Executing, RunStatus::Paused),
            (RunStatus::Executing, RunStatus::Completed),
            (RunStatus::Executing, RunStatus::Failed),
            (RunStatus::Executing, RunStatus::Cancelled),
            (RunStatus::Paused, RunStatus::Executing),
            (RunStatus::Paused, RunStatus::Cancelled),
        ];
        let all = [
            RunStatus::Planned,
            RunStatus::Approved,
            RunStatus::Executing,
            RunStatus::Paused,
            RunStatus::Completed,
            RunStatus::Failed,
            RunStatus::Cancelled,
        ];
        for from in all {
            for to in all {
                let expected = from == to || legal.contains(&(from, to));
                assert_eq!(
                    RunStatus::can_transition(from, to),
                    expected,
                    "{from:?} → {to:?}"
                );
            }
        }
    }

    #[test]
    fn the_step_state_machine_is_exact() {
        let legal = [
            (RunStepStatus::Pending, RunStepStatus::Running),
            (RunStepStatus::Running, RunStepStatus::Done),
            (RunStepStatus::Running, RunStepStatus::Failed),
            (RunStepStatus::Failed, RunStepStatus::Running),
        ];
        let all = [
            RunStepStatus::Pending,
            RunStepStatus::Running,
            RunStepStatus::Done,
            RunStepStatus::Failed,
        ];
        for from in all {
            for to in all {
                let expected = from == to || legal.contains(&(from, to));
                assert_eq!(
                    RunStepStatus::can_transition(from, to),
                    expected,
                    "{from:?} → {to:?}"
                );
            }
        }
        assert!(RunStepStatus::Pending.is_initial());
        assert!(RunStepStatus::Running.is_initial());
        assert!(!RunStepStatus::Done.is_initial());
        assert!(!RunStepStatus::Failed.is_initial());
    }
}
