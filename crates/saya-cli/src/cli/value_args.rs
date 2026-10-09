use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ClaimKindArg {
    Description,
    Alias,
    Grain,
    ColumnDescription,
    ColumnRole,
    TimeColumn,
}

impl ClaimKindArg {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Description => "description",
            Self::Alias => "alias",
            Self::Grain => "grain",
            Self::ColumnDescription => "column-description",
            Self::ColumnRole => "column-role",
            Self::TimeColumn => "time-column",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ForgetReasonArg {
    UserRequest,
    Incorrect,
    Obsolete,
    Privacy,
}

/// The task `config doctor --task` reports readiness for: `query`, `ai`, or
/// `coding`. Clap's mirror of `config::doctor_task::DoctorTask`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DoctorTaskArg {
    Query,
    Ai,
    Coding,
}

/// The decision a `/confirm`, `/reject`, or `/use` short-reference command
/// carries, resolved by `run_contracts` against the stored claim the prefix
/// names. Spec D. `Confirm` and `Reject` reach the existing mutating ops; `UseOnce`
/// reaches `use_candidate_once`, which validates and admits for one recall
/// without promoting — a candidate stays a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ReviewDecisionArg {
    Confirm,
    Reject,
    UseOnce,
}
