//! One native coding application owner shared by terminal and tool adapters.
mod approval;
mod approval_audit;
mod cancel_poll;
pub(crate) mod checks;
pub(crate) mod cli;
mod cli_mutations;
mod completion_view;
mod doctor;
mod doctor_budget;
mod doctor_extensions;
mod doctor_probe;
mod workspace_delete;
mod worktrees;
pub use cli::dispatch as dispatch_coding;
/// The doctor's one synthetic probe tool name. Re-exported under `cfg(test)`
/// so the allow-list drift test reads the production constant as its one
/// exempt name.
#[cfg(test)]
pub(crate) use doctor_probe::READINESS_PROBE_TOOL;
pub(crate) use worktrees::verify_root_checkout;
mod child_composition;
mod child_controls;
mod controls;
mod create;
pub(crate) mod database;
mod delegate;
mod delegate_batch;
mod follow_up;
mod helper_shape;
mod joined_accounting;
pub(crate) mod lane;
pub(crate) mod lifecycle;
pub(crate) mod limits;
pub use follow_up::{ChildFollowUpRequest, ChildFollowUpStarted};
mod diagnostics;
mod export;
mod failure;
mod finish_direct;
mod in_place_changes;
mod in_place_start;
mod in_place_status;
mod observations;
mod operations;
mod outcome;
mod parent;
mod prepare;
mod prepare_direct;
mod prepare_receipt;
mod progress_event;
mod progress_report;
mod project_context;
mod readiness;
mod readiness_cache;
mod readiness_direct;
mod usage;
pub use delegate::CodingTask;
mod assignment_added;
mod context;
mod continuation;
mod continuation_findings;
mod convergence;
mod host;
mod host_root;
mod in_place_lease;
mod integrate;
mod notes;
mod oneshot;
pub(crate) mod oneshot_cli;
mod prompts;
mod proposal;
mod provider_binding;
mod readiness_steps;
mod reconcile;
mod request;
mod rewind;
mod root_checkout;
mod root_direct;
pub(crate) mod root_mode;
mod setup;
mod task_flow;
mod task_history;
mod task_merge;
mod task_merge_record;
mod transcript;
pub use proposal::CodingApprovalRequest;
mod review;
mod review_diff;
mod review_in_place;
pub use review::CodingReview;
mod extension_observer;
mod extensions;
mod known_commit;
mod known_mutation;
mod owner_failure;
mod post_edit;
mod runtime;
pub use approval::CodingAuthorization;
pub use host::CodingApplication;
pub(crate) use known_mutation::{KnownWorkspaceMutation, KnownWorkspaceMutationOperation};
pub use oneshot::{
    LaunchPlan, OneShotMode, OneShotRequest, OneShotResult, consent_implied,
    finish as oneshot_finish, launch_plan, launch_write_scope, resolve_mode,
    start as oneshot_start,
};
pub(crate) use owner_failure::NativeOwnerFailureState;
pub use request::CodingJobRequest;
pub use runtime::NativeCodingJob;

pub(crate) use in_place_changes::job_changes;
pub(crate) use root_checkout::{RootChange, checkout_changes, read_root_checkout};

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
        "fresh approval for this exact job and scope is required: scripts pass --approve for exactly that scope; interactive sessions answer with `/approve <job>`"
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
    #[error("job is not owned by this application or has no active runtime")]
    NotOwned,
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
        "the model hit its output-token limit{} before answering; a reasoning model can spend the whole limit on reasoning: raise ai.max_output_tokens in trusted configuration, then resume or restart the job",
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
    Diagnostic(diagnostics::Fault),
    #[error("job failed ({}) for agent {:?}, invocation {:?}; inspect `saya job events <job>`{}", .0.class.as_str(), .0.agent_id, .0.invocation_id, failure_guidance(.0))]
    Failed(Box<saya_types::CodingRunFailure>),
    #[error(
        "authoritative terminal state could not be confirmed; primary failure: {primary:?}; inspect `saya job events <job>` before resuming"
    )]
    TerminalUnconfirmed {
        primary: Option<Box<saya_types::CodingRunFailure>>,
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

#[cfg(all(test, unix))]
#[path = "failure_guidance_tests.rs"]
mod failure_guidance_tests;

fn failure_guidance(failure: &saya_types::CodingRunFailure) -> String {
    // A stated guard carries its own one-line message naming the key to
    // change; it is more specific than any class-wide advice.
    if let Some(message) = &failure.message {
        return format!("; {message}");
    }
    match failure.class {
        saya_types::CodingFailureClass::ProviderError => {
            "; check the configured model with `saya job setup --probe`".to_owned()
        }
        saya_types::CodingFailureClass::ProviderStreamEnded => {
            "; the provider's response stream ended before it finished, most likely at a gateway or provider response time limit or because ai.max_output_tokens is too large to finish inside it: raise the gateway's streaming time limit or lower ai.max_output_tokens".to_owned()
        }
        _ => String::new(),
    }
}

mod workspace_search;

#[cfg(all(test, unix))]
#[path = "request_fixture.rs"]
mod request_fixture;

#[cfg(all(test, unix))]
#[path = "request_mode_tests.rs"]
mod request_mode_tests;

#[cfg(all(test, unix))]
#[path = "plan_gate_tests.rs"]
mod plan_gate_tests;

#[cfg(all(test, unix))]
#[path = "tools_allow_tests.rs"]
mod tools_allow_tests;

#[cfg(all(test, unix))]
#[path = "root_checkout_tests.rs"]
mod root_checkout_tests;

#[cfg(all(test, unix))]
#[path = "delegate_direct_tests/fixture.rs"]
mod delegate_direct_fixture;

#[cfg(all(test, unix))]
#[path = "delegate_direct_tests.rs"]
mod delegate_direct_tests;

#[cfg(all(test, unix))]
#[path = "delegate_validation_tests.rs"]
mod delegate_validation_tests;

#[cfg(all(test, unix))]
#[path = "direct_root_fixture.rs"]
mod direct_root_fixture;

#[cfg(all(test, unix))]
#[path = "task_merge_summary_tests.rs"]
mod task_merge_summary_tests;

#[cfg(all(test, unix))]
#[path = "task_flow_concurrency_tests.rs"]
mod task_flow_concurrency_tests;

#[cfg(all(test, unix))]
#[path = "root_retained_tests.rs"]
mod root_retained_tests;

#[cfg(all(test, unix))]
#[path = "root_agentless_delete_tests.rs"]
mod root_agentless_delete_tests;

#[cfg(all(test, unix))]
#[path = "operator_tests.rs"]
mod operator_tests;

#[cfg(all(test, unix))]
#[path = "retired_text_tests.rs"]
mod retired_text_tests;
