//! The native coding owner's one error: every refusal it states, in the words
//! the surface shows the operator.
//!
//! The type is re-exported from the module above, so every caller keeps saying
//! `CodingError`; only where the statements live changed.
use super::diagnostics::Fault;
use saya_types::{CodingFailureClass, CodingRunFailure};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CodingError {
    #[error("this host could not prove isolated command confinement; no process was started")]
    ConfinementUnavailable,
    #[error("candidate changed after review; review it again before applying")]
    StaleReview,
    #[error("job request is invalid or exceeds its bound")]
    InvalidRequest,
    #[error("invalid `--write` path set: {detail}")]
    WritePaths { detail: String },
    #[error("unknown subagent_type {requested:?}: this job's helper types are {known}")]
    UnknownSubagentType { requested: String, known: String },
    #[error(transparent)]
    ToolsAllow(#[from] saya_config::ToolsAllowError),
    #[error(
        "`--tools` on `/start` may only narrow the tools this session advertises: {names} \
         would widen it past the session's own list ({session})"
    )]
    ToolsOutsideSession { names: String, session: String },
    #[error(
        "child task assignment exceeds the assignment byte cap; every child already receives the operator request verbatim, so reference requirement files by path instead of restating them"
    )]
    AssignmentTooLong,
    #[error(
        "prepared candidate state is incomplete, malformed, or stale; inspect the private job records and start a fresh approved job if it cannot be recovered"
    )]
    PreparedCandidateState,
    #[error(
        "fresh approval for this exact job and scope is required: scripts pass --approve for exactly that scope; interactive sessions answer with `/job approve <job>`"
    )]
    ApprovalRequired,
    #[error(
        "`--plan` needs an operator to answer the submitted plan, and this run has no interactive \
         surface to put it to: start the job from the interactive panel, or drop `--plan`"
    )]
    PlanNeedsOperator,
    #[error(
        "`--plan` gates a write surface, and this job states no `--write` path: state the paths it \
         may edit, or drop `--plan`"
    )]
    PlanNeedsWritePath,
    #[error(
        "`--plan` opens the write surface through `submit_plan`, and this job's `[tools] allow` \
         list leaves that tool out, so the plan posture could never end: keep `submit_plan` in the \
         list, or drop `--plan`"
    )]
    PlanNeedsGateTool,
    #[error("job state must be private, canonical, and outside the repository")]
    StateDirectory,
    #[error(
        "this in-place job selects write tools but states no `--write` path: state the paths it \
         may edit, or `.` for the whole folder, or select read-only tools"
    )]
    InPlaceNeedsWriteScope,
    #[error(
        "in-place jobs need a plain folder, but {reason}: choose a folder below your home \
         directory, not the home directory or saya's state directory"
    )]
    InPlaceUnsupportedRoot { reason: &'static str },
    #[error(
        "this in-place job was created by a development build with a different baseline shape; \
         start a new job"
    )]
    InPlaceLegacyBaseline,
    #[error("the in-place folder was refused: {detail}; choose another folder or fix the path")]
    InPlaceRoot { detail: String },
    #[error(
        "the in-place baseline could not be captured: {detail}; narrow the folder or choose \
         another folder"
    )]
    InPlaceBaseline { detail: String },
    #[error(
        "`{operation}` needs a Git working tree, and this job works in place over a plain folder: \
         start the job inside a Git repository without `--in-place`"
    )]
    InPlaceRequiresGit { operation: &'static str },
    #[error(
        "another job is already running in this folder; wait for it to finish before starting a new one"
    )]
    InPlaceRootBusy,
    #[error(
        "`--checkout` needs a Git working tree to copy, and this folder is not one: run the job in \
         place with `--in-place`, or start it inside a Git repository"
    )]
    CheckoutNeedsRepository,
    #[error("job is not owned by this application or has no active runtime")]
    NotOwned,
    #[error("job {job} {detail}")]
    NotResumable { job: String, detail: String },
    #[error(
        "the job is still running, or was interrupted before it finished; rewind it only once the run has ended"
    )]
    RewindWhileRunning,
    #[error("this job has no working checkout to rewind")]
    RewindNoCheckout,
    #[error(
        "this job has no checkpoint {turn}: list them with `saya job rewind <job>`, and rewind to one it shows"
    )]
    RewindUnknownTurn { turn: u32 },
    #[error("this job has no checkpoint to rewind to")]
    RewindNoCheckpoint,
    #[error(
        "the working checkout changed more than one checkpoint may hold, so no checkpoint was recorded for that turn"
    )]
    RewindCaptureTooLarge,
    #[error("the job's baseline exceeds the rewind bounds, so nothing was restored")]
    RewindBaselineTooLarge,
    #[error("a job is already running")]
    Busy,
    #[error("child work is still active; await children before preparing a candidate")]
    ChildrenActive,
    #[error(
        "the unresolved effects changed since the list was made: run `saya job reconcile <job>` again and use the new plan digest"
    )]
    ReconciliationStale,
    #[error("effects cannot be reconciled: {0}")]
    ReconciliationRefused(String),
    #[error(
        "the job's model run failed and the job is recorded as failed: read its durable record with `saya job events <job>`, and check the configured model with `saya job setup --probe`"
    )]
    RunFailed,
    #[error(
        "the provider's response was truncated{} before answering; a reasoning model can spend the whole limit on reasoning: raise ai.max_output_tokens in trusted configuration, then resume or restart the job",
        configured.map(|configured| format!(" (ai.max_output_tokens = {configured})")).unwrap_or_default()
    )]
    OutputLimit { configured: Option<u32> },
    #[error(
        "the configured extension program is not usable: it must be a real executable, not a symlink, directly inside the trusted program directory; point the profile at the real file"
    )]
    ExtensionProgramRefused,
    #[error("direct-mode lease refused: {0}")]
    LeaseRefused(String),
    #[error(
        "the working checkout has no changes to prepare; the coordinator has not written anything in its working checkout"
    )]
    NothingToPrepare,
    #[error("job diagnostics failed: {0}")]
    Diagnostic(Fault),
    #[error("job failed ({}) for agent {:?}, invocation {:?}; inspect `saya job events <job>`{}", .0.class.as_str(), .0.agent_id, .0.invocation_id, failure_guidance(.0))]
    Failed(Box<CodingRunFailure>),
    #[error(
        "authoritative terminal state could not be confirmed; primary failure: {primary:?}; inspect `saya job events <job>` before resuming"
    )]
    TerminalUnconfirmed {
        primary: Option<Box<CodingRunFailure>>,
    },
    #[error(
        "job operation failed: {0}; run `saya job events <job>` or read the log for the full detail"
    )]
    Owner(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl CodingError {
    pub(crate) fn owner(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Owner(Box::new(error))
    }

    pub(crate) fn write_paths(
        error: saya_harness::coding::integration::AssignedPathsError,
    ) -> Self {
        Self::WritePaths {
            detail: error.to_string(),
        }
    }
}

fn failure_guidance(failure: &CodingRunFailure) -> String {
    // A stated guard carries its own one-line message naming the key to
    // change; it is more specific than any class-wide advice.
    if let Some(message) = &failure.message {
        return format!("; {message}");
    }
    match failure.class {
        CodingFailureClass::ProviderError => {
            "; check the configured model with `saya job setup --probe`".to_owned()
        }
        CodingFailureClass::ProviderStreamEnded => {
            "; the provider's response stream ended before it finished, most likely at a gateway or provider response time limit or because ai.max_output_tokens is too large to finish inside it: raise the gateway's streaming time limit or lower ai.max_output_tokens".to_owned()
        }
        _ => String::new(),
    }
}
